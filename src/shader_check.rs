use naga::valid::{Capabilities, ValidationFlags};

/// Parses and validates `source` as WGSL, panicking with naga's report on the first problem.
pub(crate) fn validate(name: &str, source: &str) {
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|error| panic!("{name} does not parse:\n{}", error.emit_to_string(source)));
    naga::valid::Validator::new(ValidationFlags::all(), Capabilities::default())
        .validate(&module)
        .unwrap_or_else(|error| panic!("{name} does not validate:\n{}", error.emit_to_string(source)));
}
