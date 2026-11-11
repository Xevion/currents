//! What the viewer's tests share.

use crate::settings::{CATALOG, Settings, limits};

/// One change a person can make in the menu.
pub(crate) type Step = (String, Box<dyn Fn(&mut Settings)>);

/// Settings as far as the menu takes each of them, one change at a time, ending back at the
/// defaults.
pub(crate) fn extremes() -> Vec<Step> {
    let mut steps: Vec<Step> = vec![
        ("supersampling up".into(), Box::new(|s| s.render.supersampling = *limits::SUPERSAMPLING.end())),
        ("field step up".into(), Box::new(|s| s.render.field_step = *limits::FIELD_STEP.end())),
        ("temporal off".into(), Box::new(|s| s.render.temporal = false)),
        ("pixel cap at the top".into(), Box::new(|s| s.render.max_pixels = Some(*limits::MAX_PIXELS.end()))),
        ("pixel cap at the bottom".into(), Box::new(|s| s.render.max_pixels = Some(*limits::MAX_PIXELS.start()))),
        ("render scale floor".into(), Box::new(|s| s.render.render_scale = *limits::RENDER_SCALE.start())),
        ("adaptive on".into(), Box::new(|s| s.render.adaptive = true)),
        ("frame rate limited".into(), Box::new(|s| s.render.fps_cap = Some(*limits::FPS_CAP.start()))),
        ("frame rate free".into(), Box::new(|s| s.render.fps_cap = None)),
        ("pixel cap off".into(), Box::new(|s| s.render.max_pixels = None)),
        ("supersampling down".into(), Box::new(|s| s.render.supersampling = 1)),
        ("temporal on".into(), Box::new(|s| s.render.temporal = true)),
        ("speed top".into(), Box::new(|s| s.motion.speed = *limits::SPEED.end())),
        ("camera locked".into(), Box::new(|s| s.camera.locked = true)),
        ("give at the top".into(), Box::new(|s| s.camera.give = *limits::GIVE.end())),
        ("no pause when hidden".into(), Box::new(|s| s.wallpaper.pause_when_hidden = false)),
        ("no pause under fullscreen".into(), Box::new(|s| s.wallpaper.pause_under_fullscreen = false)),
        ("pause on battery".into(), Box::new(|s| s.wallpaper.pause_on_battery = true)),
        (
            "battery frame rate limited".into(),
            Box::new(|s| s.wallpaper.battery_fps_cap = Some(*limits::FPS_CAP.start())),
        ),
        ("battery frame rate free".into(), Box::new(|s| s.wallpaper.battery_fps_cap = None)),
    ];
    for spec in &CATALOG {
        let slot = spec.slot;
        steps.push((format!("{} at {}", spec.key, spec.min), Box::new(move |s| *s.params.get_mut(slot) = spec.min)));
        steps.push((format!("{} at {}", spec.key, spec.max), Box::new(move |s| *s.params.get_mut(slot) = spec.max)));
    }
    steps.push(("everything back".into(), Box::new(|s| *s = Settings::default())));
    steps
}
