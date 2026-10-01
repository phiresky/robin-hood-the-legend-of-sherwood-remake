import { Show } from "solid-js";
import type { Level3D } from "@rle/shared";

const defaults = { enabled: true, sunAzimuth: 305, sunElevation: 48, shadowOpacity: 1 };
export default function LightingPanel(props: {
  document: () => Level3D | null;
  commit(document: Level3D): void;
}) {
  const settings = () => props.document()?.lighting ?? { ...defaults, enabled: false };
  function patch(values: Partial<typeof defaults>) {
    const document = props.document();
    if (document) props.commit({ ...document, lighting: { ...settings(), ...values } });
  }
  return (
    <section class="view-settings">
      <h2>Sun &amp; shadows</h2>
      <label class="check">
        <input
          type="checkbox"
          aria-label="Cast sun shadows"
          checked={settings().enabled}
          onChange={(e) => patch({ enabled: e.currentTarget.checked })}
        />{" "}
        Cast shadows on terrain
      </label>
      <Show when={settings().enabled}>
        <label>
          Sun direction · {settings().sunAzimuth}°
          <input
            aria-label="Sun direction"
            type="range"
            min="0"
            max="360"
            step="5"
            value={settings().sunAzimuth}
            onChange={(e) => patch({ sunAzimuth: Number(e.currentTarget.value) })}
          />
        </label>
        <label>
          Sun elevation · {settings().sunElevation}°
          <input
            aria-label="Sun elevation"
            type="range"
            min="10"
            max="85"
            step="1"
            value={settings().sunElevation}
            onChange={(e) => patch({ sunElevation: Number(e.currentTarget.value) })}
          />
        </label>
        <label>
          Shadow strength · {Math.round(settings().shadowOpacity * 100)}%
          <input
            aria-label="Shadow strength"
            type="range"
            min="0"
            max="1"
            step=".05"
            value={settings().shadowOpacity}
            onChange={(e) => patch({ shadowOpacity: Number(e.currentTarget.value) })}
          />
        </label>
        <p class="hint">Direction is clockwise from north. Lower sun casts longer shadows.</p>
      </Show>
    </section>
  );
}
