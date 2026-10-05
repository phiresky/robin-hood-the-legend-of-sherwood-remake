import { StateAppearancePlayer, type StateAppearanceTemplate } from "./state-appearance-player.ts";

/** User-controlled preview timing, independent of mission action semantics. */
export class AssetPreviewPlayback {
  readonly player: StateAppearancePlayer;
  readonly clips: readonly { name: string; lastTick: number }[];
  private selected: string;
  private repeating = false;

  constructor(template: StateAppearanceTemplate) {
    this.clips = template.clips.map((clip) => ({
      name: clip.name,
      lastTick: Math.max(...clip.tracks.map((binding) => binding.track.times.at(-1)!)),
    }));
    this.player = new StateAppearancePlayer(template);
    this.selected = this.clips[0]!.name;
    this.configure();
  }
  get asset() {
    return this.player.object;
  }
  get clip() {
    return this.selected;
  }
  get loop() {
    return this.repeating;
  }
  get lastTick() {
    return this.clips.find((clip) => clip.name === this.selected)!.lastTick;
  }
  get tick() {
    return this.player.tick;
  }
  get playing() {
    return this.player.playing;
  }
  select(name: string) {
    if (!this.clips.some((clip) => clip.name === name))
      throw new Error(`Unknown preview clip: ${name}`);
    this.selected = name;
    this.configure();
  }
  setLoop(loop: boolean) {
    const tick = this.tick;
    this.repeating = loop;
    this.configure();
    this.player.seek(tick);
  }
  seek(tick: number) {
    this.player.pause();
    this.player.seek(tick);
  }
  play() {
    if (!this.loop && this.tick === this.lastTick) this.player.seek(0);
    this.player.play();
  }
  pause() {
    this.player.pause();
  }
  advance(seconds: number) {
    const tick = this.tick,
      playing = this.playing;
    this.player.advance(seconds);
    return tick !== this.tick || playing !== this.playing;
  }
  dispose() {
    this.player.dispose();
  }
  private configure() {
    this.player.select(
      this.selected,
      this.repeating
        ? { mode: "loop", cycleTicks: this.lastTick + 1 }
        : { mode: "clamp", terminalTick: this.lastTick },
    );
  }
}

/** One animation-frame owner for all visible cards sharing a preview renderer. */
export class AssetPreviewPlaybackClock {
  private readonly entries = new Map<AssetPreviewPlayback, () => void>();
  private frame: number | undefined;
  private previous: number | undefined;
  private disposed = false;
  private readonly requestFrame: (callback: (time: number) => void) => number;
  private readonly cancelFrame: (handle: number) => void;
  constructor(
    requestFrame: (callback: (time: number) => void) => number = (callback) =>
      requestAnimationFrame(callback),
    cancelFrame: (handle: number) => void = (handle) => cancelAnimationFrame(handle),
  ) {
    this.requestFrame = requestFrame;
    this.cancelFrame = cancelFrame;
  }
  register(playback: AssetPreviewPlayback, changed: () => void) {
    if (this.disposed) throw new Error("Preview clock is disposed");
    if (this.entries.has(playback)) throw new Error("Preview is already registered");
    this.entries.set(playback, changed);
    return () => {
      this.entries.delete(playback);
      if (![...this.entries.keys()].some((entry) => entry.playing)) this.stop();
    };
  }
  request() {
    if (this.disposed) throw new Error("Preview clock is disposed");
    if (this.frame !== undefined || ![...this.entries.keys()].some((entry) => entry.playing))
      return;
    this.frame = this.requestFrame((time) => {
      this.frame = undefined;
      const seconds = this.previous === undefined ? 0 : Math.max(0, time - this.previous) / 1000;
      this.previous = time;
      for (const [entry, changed] of this.entries)
        if (entry.playing && entry.advance(seconds)) changed();
      if ([...this.entries.keys()].some((entry) => entry.playing)) this.request();
      else this.stop();
    });
  }
  dispose() {
    this.stop();
    this.entries.clear();
    this.disposed = true;
  }
  private stop() {
    if (this.frame !== undefined) this.cancelFrame(this.frame);
    this.frame = undefined;
    this.previous = undefined;
  }
}
