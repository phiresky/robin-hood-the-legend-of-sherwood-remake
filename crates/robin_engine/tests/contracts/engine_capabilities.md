
## Compiler-checked capability boundary

These examples compile against the actual public crate, not a mock type or a
second copy of its source. The positive controls ensure the types and supported
operations are accessible; each negative example isolates one forbidden access.

Read-only phases may query the facade and its borrowed projection:

```no_run
use robin_engine::engine::{Engine, EngineInner};
fn query(engine: &Engine) -> u32 {
    let inner: &EngineInner = engine;
    let _campaign = engine.campaign();
    inner.frame_counter()
}
```

An owner may admit a complete frame:

```no_run
use robin_engine::engine::{Engine, LevelAssets, SimulationFrameInput};
fn admit(engine: &mut Engine, assets: &LevelAssets) {
    engine.advance_frame(assets, SimulationFrameInput::no_hourglass())
        .expect("valid frame admission");
}
```

A shared phase cannot turn that query reference into frame-mutation authority:

```compile_fail,E0596
use robin_engine::engine::{Engine, LevelAssets, SimulationFrameInput};
fn forbidden(engine: &Engine, assets: &LevelAssets) {
    engine.advance_frame(assets, SimulationFrameInput::no_hourglass()).unwrap();
}
```

Even an owner cannot obtain a mutable projection through dereferencing:

```compile_fail,E0596
use robin_engine::engine::{Engine, EngineInner};
fn forbidden(engine: &mut Engine) -> &mut EngineInner {
    &mut **engine
}
```

The wrapped state is private:

```compile_fail,E0616
use robin_engine::engine::Engine;
fn forbidden(engine: &Engine) {
    let _ = &engine.inner;
}
```

The projection cannot be independently constructed:

```compile_fail,E0599
use robin_engine::engine::EngineInner;
let _ = EngineInner::new();
```

Low-level mutation and legacy test adapters are not downstream entry points.
Each access is a separate compiler check so one missing method cannot conceal
another accidentally exposed method. The companion AST allowlist guards new
methods, constructors and ownership escapes that these examples do not name.

```compile_fail,E0599
use robin_engine::engine::Engine;
let _ = Engine::apply_command;
```

```compile_fail,E0599
use robin_engine::engine::Engine;
let _ = Engine::apply_commands;
```

```compile_fail,E0599
use robin_engine::engine::Engine;
let _ = Engine::perform_hourglass;
```

```compile_fail,E0599
use robin_engine::engine::Engine;
let _ = Engine::perform_post_initialize;
```

```compile_fail,E0599
use robin_engine::engine::Engine;
let _ = Engine::run_console_command;
```

```compile_fail,E0599
use robin_engine::engine::Engine;
let _ = Engine::perform_hourglass_with_body_gate;
```

```compile_fail,E0599
use robin_engine::engine::Engine;
let _ = Engine::apply_external_director_completion;
```

```compile_fail,E0599
use robin_engine::engine::Engine;
let _ = Engine::queue_replay_resolved_exclamations;
```

```compile_fail,E0599
use robin_engine::engine::Engine;
let _ = Engine::apply_replay_sound_boundary;
```

```compile_fail,E0624
use robin_engine::engine::Engine;
let _ = Engine::call_external_native_with_this;
```

```compile_fail,E0599
use robin_engine::engine::Engine;
let _ = Engine::run_cheat_string;
```

```compile_fail,E0599
use robin_engine::engine::Engine;
let _ = Engine::try_ezekiel_instakill;
```

```compile_fail,E0599
use robin_engine::engine::Engine;
let _ = Engine::send_simple_message;
```

```compile_fail,E0599
use robin_engine::engine::Engine;
let _ = Engine::replace_campaign_from_console;
```

```compile_fail,E0599
use robin_engine::engine::Engine;
let _ = Engine::inject_recorded_drop_ale_route;
```
