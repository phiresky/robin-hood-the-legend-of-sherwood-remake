//! Interpreter-specific native conversion and private guest driver.
use super::*;

pub(super) fn native_bridge_source() -> String {
    let mut source = String::from(
        "local __yield = coroutine.yield\nlocal __pack_f32 = __robin_pack_f32\nlocal __unpack_f32 = __robin_unpack_f32\n",
    );
    for definition in NATIVE_REGISTRY
        .iter()
        .filter(|definition| definition.expose_to_lua)
    {
        append_native(
            &mut source,
            definition.signature.name,
            definition.native,
            &definition.signature,
        );
    }
    for (alias, native) in SPELLFORGE_NATIVE_ALIASES {
        let canonical = robin_engine::natives::native_signature_by_index(*native as u32)
            .expect("Spellforge alias target must have a signature")
            .name;
        source.push_str(alias);
        source.push('=');
        source.push_str(canonical);
        source.push('\n');
    }
    source
}

fn append_native(
    source: &mut String,
    name: &str,
    native: NativeFn,
    signature: &robin_engine::natives::NativeSignature,
) {
    source.push_str(name);
    source.push_str("=function(");
    for index in 0..signature.params.len() {
        if index != 0 {
            source.push(',');
        }
        source.push_str(&format!("a{}", index + 1));
    }
    source.push_str(")\nlocal w={}\n");
    for (index, parameter) in signature.params.iter().enumerate() {
        let arg = format!("a{}", index + 1);
        let slot = index + 1;
        match parameter.abi_type {
            NativeAbiType::Int | NativeAbiType::Handle => source.push_str(&format!(
                "if type({arg})~='number' or {arg}~={arg} or {arg}~=math.floor({arg}) or {arg} < -2147483648 or {arg} > 2147483647 then error('{name}: argument {slot} expects a signed 32-bit integer',2) end\nw[{slot}]={arg}\n"
            )),
            NativeAbiType::Float => source.push_str(&format!("w[{slot}]=__pack_f32({arg})\n")),
            NativeAbiType::Bool => source.push_str(&format!(
                "if type({arg})=='boolean' then w[{slot}]={arg} and 1 or 0 elseif type({arg})=='number' and ({arg}==0 or {arg}==1) then w[{slot}]={arg}==0 and 0 or 1 else error('{name}: argument {slot} expects a boolean or numeric 0/1',2) end\n"
            )),
            NativeAbiType::Void => panic!("void native parameter in {name}"),
        }
    }
    source.push_str(&format!(
        "local r=__yield('__robin_native_v1',{},w)\n",
        native as u32
    ));
    match signature.return_abi_type {
        NativeAbiType::Void => source.push_str("return nil\nend\n"),
        NativeAbiType::Bool => source.push_str("return r~=0\nend\n"),
        NativeAbiType::Float => source.push_str("return __unpack_f32(r)\nend\n"),
        NativeAbiType::Int | NativeAbiType::Handle => source.push_str("return r\nend\n"),
    }
}

pub(super) const BOOTSTRAP: &str = include_str!("bootstrap.lua");
