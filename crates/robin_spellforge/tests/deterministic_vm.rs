use robin_engine::spellforge::{
    SPELLFORGE_CONTRACT_VERSION, SpellforgeInvocation, SpellforgePackage, SpellforgeRuntime,
    SpellforgeScriptMode, SpellforgeStep, SpellforgeTape, SpellforgeTarget,
};
use robin_spellforge::{SpellforgeRuntime51, compute_package_sha256, spellforge_vm_abi};
use std::collections::BTreeMap;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test;

const SOURCE: &str = r#"
local values = {
    tonumber("0.10000000000000001"),
    tonumber("-1.2345678901234567e-123"),
    math.sin(0.5),
    math.cos(0.5),
    math.tan(0.5),
    math.sinh(0.5),
    math.cosh(0.5),
    math.tanh(0.5),
    math.asin(0.5),
    math.acos(0.5),
    math.atan(0.5),
    math.atan2(-0.5, 0.25),
    math.sqrt(2),
    math.exp(0.5),
    math.log(0.5),
    math.log10(0.5),
    math.pow(2, 0.5),
    2 ^ 0.5,
    -5.5 % 2.25,
    math.fmod(-5.5, 2.25),
    math.ldexp(0.75, 37),
    math.rad(123.5),
    math.deg(2.25),
    ("Z" < "a") and 1 or 0,
    string.byte(string.lower("Az"), 2),
    (tonumber("1é") == nil) and 1 or 0,
    (tonumber("1\255") == nil) and 1 or 0,
}
for index, value in ipairs(values) do
    values[index] = string.format("%.17g", value)
end
function ValueCount() return table.getn(values) end
function ValueLength(index) return string.len(values[index]) end
function ValueByte(index, offset) return string.byte(values[index], offset) end
"#;

fn package() -> SpellforgePackage {
    let mut package = SpellforgePackage {
        contract_version: SPELLFORGE_CONTRACT_VERSION,
        vm_abi: spellforge_vm_abi().to_owned(),
        script_mode: SpellforgeScriptMode::Replace,
        entrypoint: "mission.lua".to_owned(),
        files: BTreeMap::from([("mission.lua".to_owned(), SOURCE.as_bytes().to_vec())]),
        sha256: [0; 32],
    };
    package.sha256 = compute_package_sha256(&package);
    package
}

fn invoke(
    runtime: &SpellforgeRuntime51,
    tape: &mut SpellforgeTape,
    event: &str,
    args: Vec<i32>,
) -> i32 {
    let invocation = SpellforgeInvocation {
        target: SpellforgeTarget::Global,
        event: event.to_owned(),
        args,
        script_this: 0,
        current_scroll: 0,
    };
    let SpellforgeStep::Complete { activation, result } = runtime
        .begin(invocation, tape)
        .expect("guest event must run")
    else {
        panic!("determinism probe unexpectedly called an engine native")
    };
    runtime
        .commit(activation, result, tape)
        .expect("guest event must commit");
    result
}

fn run_probe() -> Vec<String> {
    let runtime = SpellforgeRuntime51::new(package()).expect("probe package must load");
    let mut tape = SpellforgeTape::default();
    tape.initialize(runtime.package().clone())
        .expect("probe tape must initialize");
    let count = invoke(&runtime, &mut tape, "ValueCount", Vec::new());
    (1..=count)
        .map(|index| {
            let length = invoke(&runtime, &mut tape, "ValueLength", vec![index]);
            let bytes = (1..=length)
                .map(|offset| invoke(&runtime, &mut tape, "ValueByte", vec![index, offset]) as u8)
                .collect();
            String::from_utf8(bytes).expect("formatted number must be ASCII")
        })
        .collect()
}

// Seventeen significant decimal digits uniquely identify every finite f64.
// Running this same golden through wasm-bindgen therefore covers exact numeric
// results as well as parsing, collation, case mapping, and formatting.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn executable_semantics_match_cross_target_golden() {
    let actual = run_probe();
    assert_eq!(
        actual,
        [
            "0.10000000000000001",
            "-1.2345678901234567e-123",
            "0.47942553860420301",
            "0.87758256189037276",
            "0.54630248984379048",
            "0.52109530549374738",
            "1.1276259652063807",
            "0.46211715726000974",
            "0.52359877559829893",
            "1.0471975511965979",
            "0.46364760900080609",
            "-1.1071487177940904",
            "1.4142135623730951",
            "1.6487212707001282",
            "-0.69314718055994529",
            "-0.3010299956639812",
            "1.4142135623730951",
            "1.4142135623730951",
            "1.25",
            "-1",
            "103079215104",
            "2.155481626212997",
            "128.91550390443521",
            "1",
            "122",
            "1",
            "1",
        ]
    );
}
