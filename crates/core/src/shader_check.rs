use naga::back::glsl;
use naga::valid::{Capabilities, ModuleInfo, ValidationFlags};

fn parse(name: &str, source: &str) -> (naga::Module, ModuleInfo) {
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|error| panic!("{name} does not parse:\n{}", error.emit_to_string(source)));
    let info = naga::valid::Validator::new(ValidationFlags::all(), Capabilities::default())
        .validate(&module)
        .unwrap_or_else(|error| panic!("{name} does not validate:\n{}", error.emit_to_string(source)));
    (module, info)
}

/// Parses and validates `source` as WGSL, panicking with naga's report on the first problem.
pub(crate) fn validate(name: &str, source: &str) {
    parse(name, source);
}

/// The value of the constant `name` as the embedded shaders declare it, if one does.
pub(crate) fn declared(name: &str) -> Option<f64> {
    let sources = crate::shaders::Sources::embedded();
    let prefix = format!("const {name} = ");
    [&sources.common, &sources.field, &sources.shade]
        .into_iter()
        .flat_map(|text| text.lines())
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|rest| rest.split(';').next())
        .and_then(|number| number.trim().parse().ok())
}

/// Translates every entry point of `source` to GLSL ES 3.00 the way the WebGL2 backend does,
/// with `constants` filling its overrides; panics if any stage fails.
pub(crate) fn to_webgl(name: &str, source: &str, constants: &[(&str, f64)]) {
    let (module, info) = parse(name, source);
    let constants = constants.iter().map(|&(k, v)| (k.to_owned(), v)).collect();
    let (module, info) = naga::back::pipeline_constants::process_overrides(&module, &info, None, &constants)
        .unwrap_or_else(|error| panic!("{name}: overrides: {error}"));
    let options = glsl::Options {
        version: glsl::Version::Embedded { version: 300, is_webgl: true },
        writer_flags: glsl::WriterFlags::ADJUST_COORDINATE_SPACE,
        ..Default::default()
    };
    for entry in &module.entry_points {
        let pipeline =
            glsl::PipelineOptions { shader_stage: entry.stage, entry_point: entry.name.clone(), multiview: None };
        let mut out = String::new();
        glsl::Writer::new(&mut out, &module, &info, &options, &pipeline, naga::proc::BoundsCheckPolicies::default())
            .and_then(|mut writer| writer.write())
            .unwrap_or_else(|error| panic!("{name}::{} does not translate to GLSL ES: {error}", entry.name));
        // naga numbers a WGSL `gl` as `gl_1`, which GLSL reserves along with every other `gl_` name
        let reserved = out.match_indices("gl_").find(|&(i, _)| out[i + 3..].starts_with(|c: char| c.is_ascii_digit()));
        assert!(
            reserved.is_none(),
            "{name}::{} emits a reserved GLSL name; rename the WGSL `gl` identifier",
            entry.name
        );
    }
}
