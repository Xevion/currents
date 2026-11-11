//! Reading the settings file forgivingly and writing back only what changed, so a file someone
//! edited by hand keeps its comments, order and unknown keys.

use snafu::{ResultExt, Snafu};
pub use toml_edit::Value;
use toml_edit::{DocumentMut, Item, Table, TableLike};

use super::keys::Chord;
use super::{Issue, Settings};

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("the file is not valid TOML"))]
    Parse { source: toml_edit::TomlError },
}

type Path = Vec<String>;

/// `settings` as a TOML table, each float written at the precision it is stored in.
fn tree(settings: &Settings) -> Table {
    let mut doc = toml_edit::ser::to_document(settings).expect("settings always serialise").as_table().clone();
    normalise(&mut doc);
    doc
}

fn normalise(table: &mut dyn TableLike) {
    for (_, item) in table.iter_mut() {
        if let Some(sub) = item.as_table_like_mut() {
            normalise(sub);
        } else if let Some(Value::Float(f)) = item.as_value_mut() {
            // The settings hold f32; widening 0.4 to f64 would write 0.4000000059604645.
            let short: f64 = (*f.value() as f32).to_string().parse().unwrap_or_else(|_| *f.value());
            *f = toml_edit::Formatted::new(short);
        }
    }
}

fn leaves(table: &dyn TableLike, prefix: &mut Path, out: &mut Vec<(Path, Value)>) {
    for (key, item) in table.iter() {
        prefix.push(key.to_owned());
        if let Some(sub) = item.as_table_like() {
            leaves(sub, prefix, out);
        } else if let Some(value) = item.as_value() {
            out.push((prefix.clone(), value.clone()));
        }
        prefix.pop();
    }
}

fn leaves_of(table: &dyn TableLike) -> Vec<(Path, Value)> {
    let mut out = Vec::new();
    leaves(table, &mut Vec::new(), &mut out);
    out
}

fn lookup<'a>(table: &'a dyn TableLike, path: &[String]) -> Option<&'a Item> {
    let (last, parents) = path.split_last()?;
    let mut table = table;
    for key in parents {
        table = table.get(key)?.as_table_like()?;
    }
    table.get(last)
}

/// The value as it reads in a file, without the spacing and comments around it.
fn bare(value: &Value) -> String {
    let mut value = value.clone();
    value.decor_mut().clear();
    value.to_string()
}

/// `found` in the type `default` has, if it can be read as one.
fn conformed(found: &Value, default: &Value) -> Option<Value> {
    match (found, default) {
        (Value::Boolean(_), Value::Boolean(_))
        | (Value::Integer(_), Value::Integer(_))
        | (Value::Float(_), Value::Float(_)) => Some(found.clone()),
        (Value::Integer(i), Value::Float(_)) => Some(Value::from(*i.value() as f64)),
        (Value::String(s), Value::String(_)) => s.value().parse::<Chord>().is_ok().then(|| found.clone()),
        _ => None,
    }
}

fn describe(value: &Value) -> &'static str {
    match value {
        Value::String(_) => "text",
        Value::Integer(_) => "a whole number",
        Value::Float(_) => "a number",
        Value::Boolean(_) => "true or false",
        Value::Datetime(_) => "a date",
        Value::Array(_) => "a list",
        Value::InlineTable(_) => "a table",
    }
}

fn issue(path: &[String], message: impl Into<String>) -> Issue {
    Issue { path: path.join("."), message: message.into() }
}

/// Reads a settings file. A key with an unusable value falls back to its default and an
/// unknown key is skipped; each is reported. Only text that is not TOML at all fails.
pub fn load(text: &str) -> Result<(Settings, Vec<Issue>), Error> {
    let doc: DocumentMut = text.parse().context(ParseSnafu)?;
    // The schema holds every key a file may set, optional ones included, each with its type.
    let schema = tree(&Settings::schema());
    let mut merged = tree(&Settings::default());
    let mut issues = Vec::new();

    for (path, default) in leaves_of(&schema) {
        let Some(found) = lookup(doc.as_table(), &path) else { continue };
        let Some(found) = found.as_value().filter(|v| v.as_inline_table().is_none()) else {
            issues.push(issue(&path, format!("expected {}; using the default", describe(&default))));
            continue;
        };
        match conformed(found, &default) {
            Some(value) => set(&mut merged, &path, value),
            None if matches!(default, Value::String(_)) => {
                issues.push(issue(&path, format!("{} is not a key chord; using the default", bare(found))));
            },
            None => issues.push(issue(&path, format!("expected {}; using the default", describe(&default)))),
        }
    }
    for (path, _) in leaves_of(doc.as_table()) {
        if lookup(&schema, &path).is_none() {
            issues.push(issue(&path, "unknown setting; ignored"));
        }
    }

    let mut settings: Settings = match toml_edit::de::from_document(DocumentMut::from(merged)) {
        Ok(settings) => settings,
        Err(error) => {
            issues.push(issue(&[], format!("could not be read ({error}); using every default")));
            Settings::default()
        },
    };
    issues.extend(settings.clamp());
    Ok((settings, issues))
}

/// `settings` with the setting at dotted `path` changed to `value`, held to its bounds.
///
/// `None` switches an optional setting off. A path that names no setting, a value of the wrong
/// type, or `None` for a setting that cannot be off is refused and changes nothing.
pub fn assign(settings: &Settings, path: &str, value: Option<Value>) -> Result<(Settings, Vec<Issue>), Issue> {
    let path: Path = path.split('.').map(str::to_owned).collect();
    let schema = tree(&Settings::schema());
    let Some(default) = lookup(&schema, &path).and_then(Item::as_value) else {
        return Err(issue(&path, "unknown setting"));
    };
    let mut merged = tree(settings);
    match value {
        Some(found) => {
            let Some(value) = conformed(&found, default) else {
                return Err(issue(&path, format!("expected {}, not {}", describe(default), bare(&found))));
            };
            set(&mut merged, &path, value);
        },
        None if lookup(&tree(&Settings::default()), &path).is_none() => {
            remove(&mut merged, &path);
        },
        None => return Err(issue(&path, "cannot be switched off")),
    }
    let mut new: Settings = toml_edit::de::from_document(DocumentMut::from(merged))
        .map_err(|error| issue(&path, format!("could not be read ({error})")))?;
    let issues = new.clamp();
    Ok((new, issues))
}

/// Writes `value` at `path`, keeping the comments around a value it replaces and making the
/// tables on the way as plain `[section]` headers.
fn set(table: &mut dyn TableLike, path: &[String], value: Value) {
    let Some((last, parents)) = path.split_last() else { return };
    let mut table = table;
    for key in parents {
        let entry = table.entry(key).or_insert_with(|| Item::Table(Table::new()));
        if entry.as_table_like().is_none() {
            *entry = Item::Table(Table::new());
        }
        table = entry.as_table_like_mut().expect("made a table just above");
    }
    let mut value = value;
    if let Some(old) = table.get(last).and_then(Item::as_value) {
        *value.decor_mut() = old.decor().clone();
    } else {
        value.decor_mut().clear();
    }
    table.insert(last, Item::Value(value));
}

/// Writes into `doc` every setting that differs between `old` and `new`, and nothing else;
/// true when anything was written.
pub fn write_back(doc: &mut DocumentMut, old: &Settings, new: &Settings) -> bool {
    write(doc, Some(&tree(old)), new)
}

/// The whole of `settings` as a settings file.
pub fn to_string(settings: &Settings) -> String {
    let mut doc = DocumentMut::new();
    write(&mut doc, None, settings);
    doc.to_string()
}

fn write(doc: &mut DocumentMut, old: Option<&Table>, new: &Settings) -> bool {
    let new = tree(new);
    let mut changed = false;
    for (path, value) in leaves_of(&new) {
        let before = old.and_then(|t| lookup(t, &path)).and_then(Item::as_value).map(bare);
        if before.as_deref() != Some(bare(&value).as_str()) {
            set(doc.as_table_mut(), &path, value);
            changed = true;
        }
    }
    // An optional setting switched off is written as the key's absence.
    for (path, _) in old.map(|t| leaves_of(t)).unwrap_or_default() {
        if lookup(&new, &path).is_none() {
            changed |= remove(doc.as_table_mut(), &path);
        }
    }
    changed
}

/// The dotted paths of every setting that differs between `old` and `new`.
pub fn changed(old: &Settings, new: &Settings) -> Vec<String> {
    let (old, new) = (tree(old), tree(new));
    let value_at = |t: &Table, path: &[String]| lookup(t, path).and_then(Item::as_value).map(bare);
    let mut paths: Vec<String> = leaves_of(&new)
        .into_iter()
        .chain(leaves_of(&old))
        .filter(|(path, _)| value_at(&old, path) != value_at(&new, path))
        .map(|(path, _)| path.join("."))
        .collect();
    paths.dedup();
    paths
}

/// Removes the value at `path`; true when there was one.
fn remove(table: &mut dyn TableLike, path: &[String]) -> bool {
    let Some((last, parents)) = path.split_last() else { return false };
    let mut table = table;
    for key in parents {
        let Some(next) = table.get_mut(key).and_then(Item::as_table_like_mut) else { return false };
        table = next;
    }
    table.remove(last).is_some()
}

#[cfg(test)]
mod tests {
    use assert2::{assert, check};

    use super::*;

    const HAND_EDITED: &str = r#"# my wallpaper setup
[render]
fps_cap = 30   # laptop on battery
# temporal stays on

[motion]
speed = 0.25

[keys]
toggle_menu = "F1"

[extra]
mine = "kept"
"#;

    #[test]
    fn a_hand_edited_file_loads_what_it_says() {
        assert!(let Ok((settings, issues)) = load(HAND_EDITED));
        check!(settings.render.fps_cap == Some(30));
        check!(settings.motion.speed.total_cmp(&0.25).is_eq());
        check!(settings.keys.toggle_menu.to_string() == "F1");
        check!(issues.iter().map(|i| i.path.as_str()).collect::<Vec<_>>() == ["extra.mine"]);
    }

    #[test]
    fn writing_back_unchanged_settings_leaves_the_file_byte_for_byte() {
        let (settings, _) = load(HAND_EDITED).unwrap();
        let mut doc: DocumentMut = HAND_EDITED.parse().unwrap();
        check!(!write_back(&mut doc, &settings, &settings));
        check!(doc.to_string() == HAND_EDITED);
    }

    #[test]
    fn writing_back_touches_only_the_changed_keys() {
        let (old, _) = load(HAND_EDITED).unwrap();
        let mut new = old.clone();
        new.render.fps_cap = Some(60);
        new.motion.speed = 0.4;
        new.wallpaper.hint = false;
        let mut doc: DocumentMut = HAND_EDITED.parse().unwrap();
        check!(write_back(&mut doc, &old, &new));
        let expected = HAND_EDITED
            .replace("fps_cap = 30   # laptop on battery", "fps_cap = 60   # laptop on battery")
            .replace("speed = 0.25", "speed = 0.4")
            + "\n[wallpaper]\nhint = false\n";
        check!(doc.to_string() == expected);
        check!(load(&doc.to_string()).unwrap().0 == new);
    }

    #[test]
    fn unusable_values_fall_back_one_key_at_a_time() {
        let text =
            "[render]\nfps_cap = \"lots\"\nsupersampling = 2\n[keys]\npause = \"Ctrl+Banana\"\n[motion]\nspeed = 1\n";
        assert!(let Ok((settings, issues)) = load(text));
        check!(settings.render.fps_cap.is_none());
        check!(settings.render.supersampling == 2);
        check!(settings.keys.pause == Settings::default().keys.pause);
        check!(settings.motion.speed.total_cmp(&1.0).is_eq());
        let paths: Vec<_> = issues.iter().map(|i| i.path.as_str()).collect();
        check!(paths == ["render.fps_cap", "keys.pause"]);
    }

    #[test]
    fn switching_a_limit_off_removes_its_key_and_on_brings_it_back() {
        let (old, _) = load(HAND_EDITED).unwrap();
        let mut off = old.clone();
        off.render.fps_cap = None;
        let mut doc: DocumentMut = HAND_EDITED.parse().unwrap();
        check!(write_back(&mut doc, &old, &off));
        check!(!doc.to_string().contains("fps_cap"));
        check!(doc.to_string().contains("# temporal stays on"));
        check!(load(&doc.to_string()).unwrap().0 == off);

        let mut on = off.clone();
        on.render.max_pixels = Some(800_000.0);
        check!(write_back(&mut doc, &off, &on));
        check!(doc.to_string().contains("max_pixels = 800000.0"));
        check!(load(&doc.to_string()).unwrap().0 == on);
    }

    #[test]
    fn the_defaults_write_no_optional_keys() {
        let text = to_string(&Settings::default());
        check!(!text.contains("fps_cap") && !text.contains("max_pixels"));
        check!(load(&text).unwrap().0 == Settings::default());
    }

    /// A file the menu wrote before the limits became optional, which froze the viewer at
    /// launch; it has to load as it reads, with nothing to report.
    const WRITTEN_WITH_ZERO_FOR_NONE: &str = "[params]\nobstacle_scale = 0.21\nglare_gain = 0.3\n\n[render]\nsupersampling = 1\nadaptive = false\nmax_pixels = 800000.0\nfield_step = 1\nfps_cap = 0\n";

    #[test]
    fn a_file_with_zero_for_no_frame_rate_limit_loads_as_written() {
        assert!(let Ok((settings, issues)) = load(WRITTEN_WITH_ZERO_FOR_NONE));
        check!(issues.is_empty(), "{issues:?}");
        check!(settings.render.fps_cap.is_none());
        check!(settings.render.max_pixels == Some(800_000.0));
        check!(settings.params.get(1).total_cmp(&0.21).is_eq());
    }

    #[test]
    fn assigning_any_setting_its_own_value_changes_nothing() {
        let settings = Settings::schema();
        for (path, value) in leaves_of(&tree(&settings)) {
            let dotted = path.join(".");
            assert!(let Ok((new, issues)) = assign(&settings, &dotted, Some(value)), "{dotted}");
            check!(new == settings, "{dotted}");
            check!(issues.is_empty(), "{dotted}: {issues:?}");
        }
    }

    #[rstest::rstest]
    #[case::speed("motion.speed", Value::from(99.0), "4.0")]
    #[case::whole_number_for_a_float("motion.speed", Value::from(2), "2.0")]
    #[case::frame_rate("render.fps_cap", Value::from(1000), "240")]
    #[case::no_frames_means_no_limit("render.fps_cap", Value::from(0), "")]
    #[case::param("params.ball_spin", Value::from(-50.0), "-1.0")]
    #[case::chord("keys.toggle_menu", Value::from("F1"), "\"F1\"")]
    fn an_assigned_value_lands_inside_its_bounds(#[case] path: &str, #[case] value: Value, #[case] held: &str) {
        assert!(let Ok((new, _)) = assign(&Settings::default(), path, Some(value)));
        let path: Path = path.split('.').map(str::to_owned).collect();
        let written = lookup(&tree(&new), &path).and_then(Item::as_value).map(bare).unwrap_or_default();
        check!(written == held);
        let mut new = new;
        check!(new.clamp().is_empty());
    }

    #[rstest::rstest]
    #[case::unknown("render.sparkles", Some(Value::from(true)))]
    #[case::a_whole_table("render", Some(Value::from(true)))]
    #[case::wrong_type("render.temporal", Some(Value::from("yes")))]
    #[case::not_a_chord("keys.pause", Some(Value::from("Ctrl+Banana")))]
    #[case::cannot_be_off("motion.speed", None)]
    fn an_assignment_that_cannot_apply_is_refused(#[case] path: &str, #[case] value: Option<Value>) {
        assert!(let Err(issue) = assign(&Settings::default(), path, value));
        check!(issue.path == path);
    }

    #[test]
    fn assigning_nothing_switches_a_limit_off() {
        let mut settings = Settings::default();
        settings.render.fps_cap = Some(30);
        assert!(let Ok((new, _)) = assign(&settings, "render.fps_cap", None));
        check!(new.render.fps_cap.is_none());
        check!(new.render == crate::settings::Render { fps_cap: None, ..settings.render });
    }

    #[test]
    fn text_that_is_not_toml_is_refused() {
        check!(load("[render\nfps_cap = ").is_err());
    }

    #[test]
    fn params_are_stored_by_name() {
        let mut settings = Settings::default();
        *settings.params.get_mut(27) = 0.5;
        let text = to_string(&settings);
        check!(text.contains("ball_spin = 0.5"));
        check!(load(&text).unwrap().0 == settings);
    }
}
