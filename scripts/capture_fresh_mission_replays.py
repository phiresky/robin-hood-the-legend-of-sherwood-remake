#!/usr/bin/env python3
"""Capture deterministic fresh-mission runs; publish only complete native traces.

Pass a profile JSON exported by cpf_to_json and explicit capture/converter
binaries. Every process gets a private working directory and fresh campaign.
No savegame is loaded. Campaign progression is deliberately not randomized.
"""
import argparse
import concurrent.futures
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import threading
import time

FOOTER = struct.Struct('<16sIQQ')


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write_json(path, value):
    temporary = path.with_suffix(path.suffix + '.tmp')
    temporary.write_text(json.dumps(value, indent=2) + '\n')
    temporary.replace(path)


def mission_inventory(profile, data):
    levels = {p.stem.casefold() for p in (data / 'Data/Levels').glob('*.rhm')}
    scripts = {p.stem.casefold() for p in (data / 'Data/Levels').glob('*.scb')}
    missions = {}
    excluded = []
    for key, entry in profile['missions'].items():
        name = entry['mission_filename']
        if name.casefold() not in levels:
            excluded.append(key)
            continue
        if name.casefold() not in scripts:
            raise ValueError(f'{name}: missing mission script')
        # The default fresh campaign creates Robin. The rescue of Robin requires
        # Marian instead. Reject additional requirements rather than guessing.
        required = entry['required_character_indices']
        if required not in ([], [0], [6]):
            raise ValueError(f'{name}: unsupported required team {required}')
        value = dict(mission=name, proto=entry['proto_level_filename'],
                     team='M' if required == [6] else None)
        if name in missions and missions[name] != value:
            raise ValueError(f'{name}: conflicting mission profiles')
        missions[name] = value
    if {name.casefold() for name in missions} != levels:
        raise ValueError('playable mission files and campaign inventory differ')
    return sorted(missions.values(), key=lambda m: m['mission']), sorted(excluded)


def validate_header(header, run):
    expected = dict(type='header', schema=16, start_state='mission_start',
                    initial_frame=0, mission=run['mission'], rng_seed=run['seed'],
                    random_input_seed=run['input_seed'], simulation_hz=25)
    for key, value in expected.items():
        if header.get(key) != value:
            raise ValueError(f'header {key}: {header.get(key)!r}, expected {value!r}')
    if 'initial_save' in header or not header.get('campaign'):
        raise ValueError('fresh trace must contain a campaign and no save payload')
    if header.get('proto_level', '').casefold() != run['proto'].casefold():
        raise ValueError('wrong prototype')
    if header.get('sim_config', {}).get('difficulty', '').casefold() != run['difficulty'].casefold():
        raise ValueError('wrong difficulty')


def validate_extent(path, frames):
    with path.open('rb') as stream:
        stream.seek(-FOOTER.size, 2)
        magic, version, count, final = FOOTER.unpack(stream.read())
    if magic != b'RHPRTRACEFOOTER!' or version != 68 or count > frames or final != count:
        raise ValueError(f'invalid native trace: version={version}, frames={count}, final={final}')
    return count


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--profile', type=Path, required=True)
    parser.add_argument('--data', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--converter', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--jobs', type=int, default=8)
    parser.add_argument('--convert-jobs', type=int, default=2)
    parser.add_argument('--limit', type=int)
    parser.add_argument('--mission')
    parser.add_argument('--plan-only', action='store_true')
    parser.add_argument('--timeout', type=int, default=1800)
    args = parser.parse_args()
    if not 1 <= args.jobs <= 10 or not 1 <= args.convert_jobs <= 4:
        parser.error('jobs must be 1..10 and convert-jobs 1..4')
    for attr in ('profile', 'data', 'binary', 'converter'):
        setattr(args, attr, getattr(args, attr).resolve(strict=True))
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    # One controller owns publication/resume. Child captures remain independent.
    with (out / '.controller.lock').open('w') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        missions, excluded = mission_inventory(json.loads(args.profile.read_text()), args.data)
        runs = []
        for mission in missions:
            for number in range(1, 11):
                seed = int.from_bytes(hashlib.sha256(f"fresh-v1/{mission['mission']}/{number}".encode()).digest()[:4], 'little')
                runs.append(dict(**mission, number=number, seed=seed, input_seed=seed ^ 0x9e3779b9,
                                 difficulty=('EASY', 'MEDIUM', 'HARD')[(number - 1) % 3]))
        runs.sort(key=lambda run: (run['number'], run['mission']))
        manifest = dict(version=1, frames=1500, simulation_hz=25, runs=runs,
                        excluded_nonplayable_profiles=excluded, producer_sha256=digest(args.binary),
                        profile_sha256=digest(args.profile), converter_sha256=digest(args.converter),
                        campaign_policy='fresh default campaign; required Marian rescue team; no save load')
        path = out / 'manifest.json'
        if path.exists() and json.loads(path.read_text()) != manifest:
            raise ValueError('manifest changed; use a new corpus directory')
        write_json(path, manifest)
        (out / 'campaign.env').write_text(f'PARITY_TRACE_SCHEMA=16\nPARITY_FRAMES=1500\nEXPECTED_LOGICAL_REPLAYS={len(runs)}\nSTART_STATE=mission_start\n')
        if args.plan_only:
            print(f'{len(missions)} missions, {len(runs)} runs, {len(runs)*1500} frames')
            return
        selected = [r for r in runs if not args.mission or r['mission'] == args.mission]
        if args.mission and not selected:
            raise ValueError(f'unknown mission {args.mission}')
        if args.limit is not None:
            selected = selected[:args.limit]
        slots = threading.Semaphore(args.convert_jobs)
        env = dict(os.environ, ROBINHOOD_DATA_DIR=str(args.data), SDL_VIDEODRIVER='dummy', SDL_AUDIODRIVER='dummy')

        def capture(run):
            name = f"{run['mission']}/replay-{run['number']:03}"
            destination = out / 'traces' / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            marker = destination.with_suffix('.complete')
            native = destination.with_name(destination.name + '-session-0001.jsonl.zst.parity.bitcode.zst')
            if marker.exists():
                validate_extent(native, 1500)
                evidence = json.loads(marker.read_text())
                if digest(native) != evidence['native_sha256']:
                    raise ValueError(f'{name}: published trace checksum changed')
                return name, 'skipped'
            attempt = out / '.attempts' / name / str(time.time_ns())
            attempt.mkdir(parents=True)
            command = [str(args.binary), '-PARITYMISSION', run['mission'], run['proto'],
                       '-DIFFICULTY', run['difficulty'], '-PARITYSEED', str(run['seed']),
                       '-PARITYFRAMES', '1500', '-PARITYRANDOMINPUT', str(run['input_seed']),
                       '-PARITYTRACE', str(attempt / 'replay')]
            if run['team']:
                command += ['-PC', run['team']]
            try:
                if shutil.disk_usage(out).free < 20 * 1024**3:
                    raise RuntimeError('capture admission needs 20 GiB free; retry after freeing space')
                write_json(attempt / 'command.json', dict(command=command, run=run))
                print(f'capture {name}', flush=True)
                with (attempt / 'capture.log').open('w') as log:
                    subprocess.run(command, cwd=attempt, env=env, stdout=log, stderr=subprocess.STDOUT,
                                   timeout=args.timeout, check=True)
                traces = list(attempt.glob('replay-session-*.jsonl'))
                if len(traces) != 1:
                    raise ValueError(f'expected one fresh session, got {len(traces)}')
                trace = traces[0]
                with trace.open() as stream:
                    header = json.loads(stream.readline())
                validate_header(header, run)
                write_json(attempt / 'header.json', header)
                # Conversion audits every JSONL record and its terminal suffix,
                # then removes JSONL only after verifying the native encoding.
                with slots, (attempt / 'convert.log').open('w') as log:
                    subprocess.run([str(args.converter), '--convert', str(trace)], cwd=attempt,
                                   stdout=log, stderr=subprocess.STDOUT, timeout=args.timeout, check=True)
                artifacts = list(attempt.glob('*.parity.bitcode.zst'))
                if len(artifacts) != 1:
                    raise ValueError('conversion produced no unique native artifact')
                actual_frames = validate_extent(artifacts[0], 1500)
                checksum = digest(artifacts[0])
                artifacts[0].replace(native)
                write_json(marker, dict(run=run, native_sha256=checksum,
                                       campaign=header['campaign'], producer_sha256=manifest['producer_sha256'],
                                       frames=actual_frames, frame_limit=1500,
                                       termination='frame_limit' if actual_frames == 1500 else 'early_game_exit',
                                       attempt=str(attempt)))
                return name, 'captured'
            except Exception as error:
                write_json(attempt / 'failure.json', dict(error=str(error)))
                return name, f'failed: {error}'

        results = {}
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
            futures = [pool.submit(capture, run) for run in selected]
            for future in concurrent.futures.as_completed(futures):
                name, status = future.result()
                results[name] = status
                print(f'{status} {name} ({len(results)}/{len(selected)})', flush=True)
                write_json(out / 'latest-invocation.json', results)
        if any(status.startswith('failed') for status in results.values()):
            raise SystemExit(1)


if __name__ == '__main__':
    main()
