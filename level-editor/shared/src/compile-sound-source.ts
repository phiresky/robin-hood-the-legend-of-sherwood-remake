import type { AssetSoundSource } from "./asset-gameplay.ts";
import type { SoundSource } from "./level.ts";
import type { Vec3 } from "./scene.ts";

/** Compile an acoustic definition using its owning asset's placement. */
export function compileSoundSource(
  sound: AssetSoundSource,
  transform: (node: string, point: Vec3) => Vec3,
): SoundSource {
  const quantize = (value: number) => {
    const result = Math.round(value);
    if (!Number.isFinite(result) || result < -32768 || result > 32767)
      throw new Error(`Sound coordinate outside signed 16-bit range: ${value}`);
    return result;
  };
  const s = sound.spatial;
  // Global emitters still require their owning part to be present.
  if (!s) transform(sound.node, [0, 0, 0]);
  return {
    id: sound.sample,
    active: sound.active,
    source_kind: sound.kind,
    delayed_params: sound.delay ? [...sound.delay] : null,
    global: !s,
    ...(s?.polylineBreaks?.length ? { polyline_breaks: [...s.polylineBreaks] } : {}),
    polyline: s
      ? s.polyline.map((p) => {
          const [x, y, z] = transform(sound.node, p);
          return [quantize(x), quantize(y - z)];
        })
      : null,
    inner_distance: s?.innerDistance ?? null,
    outer_distance: s?.outerDistance ?? null,
    inner_volume: s?.innerVolume ?? null,
    outer_volume: s?.outerVolume ?? null,
    noise_covering_distance: s?.noiseCoveringDistance ?? null,
    altitude: sound.altitude,
    ambience_filter: sound.ambiences,
  };
}
