//! The files a wallpaper host reads to list the web viewer and draw its property panel:
//! Wallpaper Engine's `project.json`, Lively's `LivelyInfo.json` and `LivelyProperties.json`.

use std::fs;
use std::path::Path;

use currents_core::hosts::{Host, Kind, Prop, props};
use serde_json::{Map, Value, json};
use snafu::{ResultExt, Snafu};

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("could not write {name}"))]
    Write { source: std::io::Error, name: String },
}

const TITLE: &str = "Currents";
const DESCRIPTION: &str = "Flowing stripes parting around a chrome ball, simulated and rendered live on your GPU.";
/// The page and its still, as `web/build.py wallpaper` lays them out.
const PAGE: &str = "index.html";
const PREVIEW: &str = "preview.jpg";
/// Lively's number for a web page wallpaper, which it reads whether or not it writes names.
const LIVELY_WEB: u32 = 1;

/// Decimals a slider of `step` shows.
fn precision(step: f64) -> u32 {
    (-step.log10()).ceil().max(0.0) as u32
}

fn wallpaper_engine(props: &[Prop]) -> Value {
    let properties: Map<String, Value> = props
        .iter()
        .enumerate()
        .map(|(order, prop)| {
            let mut entry = match prop.kind {
                Kind::Slider { min, max, step, value } => json!({
                    "type": "slider", "min": min, "max": max, "step": step, "value": value,
                    "fraction": step < 1.0, "precision": precision(step),
                }),
                Kind::Toggle { value } => json!({ "type": "bool", "value": value }),
            };
            entry["order"] = json!(100 + order);
            entry["text"] = json!(prop.label);
            (prop.key.clone(), entry)
        })
        .collect();
    json!({
        "title": TITLE,
        "description": DESCRIPTION,
        "type": "web",
        "file": PAGE,
        "preview": PREVIEW,
        "tags": ["Abstract"],
        "general": { "properties": properties },
    })
}

fn lively_properties(props: &[Prop]) -> Value {
    let properties: Map<String, Value> = props
        .iter()
        .map(|prop| {
            let entry = match prop.kind {
                Kind::Slider { min, max, step, value } => json!({
                    "type": "slider", "text": prop.label, "min": min, "max": max, "step": step, "value": value,
                }),
                Kind::Toggle { value } => json!({ "type": "checkbox", "text": prop.label, "value": value }),
            };
            (prop.key.clone(), entry)
        })
        .collect();
    Value::Object(properties)
}

fn lively_info() -> Value {
    json!({
        "AppVersion": "2.0.0.0",
        "Title": TITLE,
        "Desc": DESCRIPTION,
        "Thumbnail": PREVIEW,
        "Preview": PREVIEW,
        "Type": LIVELY_WEB,
        "FileName": PAGE,
        // Asks Lively to tell the page when it pauses, so the viewer stops drawing too.
        "Arguments": "--pause-event true",
        "IsAbsolutePath": false,
    })
}

/// Each file `host` reads, by name.
pub(crate) fn files(host: Host) -> Vec<(&'static str, Value)> {
    let props = props(host);
    match host {
        Host::WallpaperEngine => vec![("project.json", wallpaper_engine(&props))],
        Host::Lively => vec![("LivelyInfo.json", lively_info()), ("LivelyProperties.json", lively_properties(&props))],
    }
}

/// Writes `host`'s files into `dir`, beside the page `web/build.py wallpaper` put there.
pub(crate) fn run(host: Host, dir: &Path) -> Result<(), Error> {
    for (name, value) in files(host) {
        let text = serde_json::to_string_pretty(&value).expect("JSON values always serialise") + "\n";
        fs::write(dir.join(name), text).context(WriteSnafu { name })?;
        tracing::info!(file = %dir.join(name).display(), "host file written");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use assert2::check;
    use rstest::rstest;

    use super::*;

    /// The files as the host reads them; a change here is a change to what each host shows.
    #[rstest]
    #[case::wallpaper_engine(Host::WallpaperEngine)]
    #[case::lively(Host::Lively)]
    fn host_files_match_the_snapshot(#[case] host: Host) {
        let mut written = String::new();
        for (name, value) in files(host) {
            writeln!(written, "== {name}\n{}", serde_json::to_string_pretty(&value).unwrap()).unwrap();
        }
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("src/host_package/{host}.snap"));
        if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
            fs::write(&path, &written).unwrap();
        }
        let expected = fs::read_to_string(&path).unwrap_or_default();
        check!(written == expected, "run with UPDATE_SNAPSHOTS=1 to accept: {}", path.display());
    }

    #[rstest]
    #[case(1.0, 0)]
    #[case(0.05, 2)]
    #[case(0.01, 2)]
    #[case(0.001, 3)]
    fn sliders_show_as_many_decimals_as_their_step(#[case] step: f64, #[case] decimals: u32) {
        check!(precision(step) == decimals);
    }
}
