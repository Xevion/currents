//! The whole viewer in a real window: surface, event loop, menu and input. These need an X
//! display and open windows on it, so they run only under `just test-display`, which gives
//! them a virtual one; they refuse to run without it rather than open windows on a desktop.

use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::{Duration, Instant};

use assert2::{assert, check};
use rstest::rstest;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{DeviceId, ElementState, MouseButton, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::KeyCode;
use winit::platform::pump_events::{EventLoopExtPumpEvents, PumpStatus};
use winit::platform::x11::EventLoopBuilderExtX11;
use winit::window::WindowId;

use super::opener::{BALL_C, BALL_R};
use super::testing::extremes;
use super::{App, Event, Hooks, Launch, Remote, app};
use crate::settings::toml::{self, Value};
use crate::settings::{Settings, limits};
use crate::shaders::Sources;

/// Set by `just test-display` once a virtual display is up.
const DISPLAY_GUARD: &str = "CURRENTS_TEST_DISPLAY";
/// Frames to present after each step before the next.
const FRAMES_PER_STEP: u64 = 4;
/// The longest a step may wait for its frames before the test fails.
const STEP_TIMEOUT: Duration = Duration::from_secs(20);

type Action = Box<dyn FnMut(&mut App, &ActiveEventLoop)>;

type Check = Box<dyn Fn(&mut App) -> Option<String>>;

/// A named step and checks of what it left behind.
struct Step {
    name: String,
    act: Action,
    /// Checked straight after the step acts, for what lasts only moments, like a toast.
    at_once: Check,
    /// Checked once the step's frames are presented; returns a problem if there is one.
    expect: Check,
    /// How long the step lasts when it waits on time instead of frames.
    lasts: Option<Duration>,
}

fn step(name: impl Into<String>, act: impl FnMut(&mut App, &ActiveEventLoop) + 'static) -> Step {
    Step { name: name.into(), act: Box::new(act), at_once: Box::new(|_| None), expect: Box::new(|_| None), lasts: None }
}

impl Step {
    fn expect(mut self, check: impl Fn(&App) -> Option<String> + 'static) -> Self {
        self.expect = Box::new(move |app| check(app));
        self
    }

    /// Makes the step wait `time` rather than for frames, for a step that expects none.
    const fn lasting(mut self, time: Duration) -> Self {
        self.lasts = Some(time);
        self
    }

    fn at_once(mut self, check: impl Fn(&mut App) -> Option<String> + 'static) -> Self {
        self.at_once = Box::new(check);
        self
    }
}

/// Whether a toast or anything else of the overlay is showing right now.
fn overlay_showing(app: &mut App) -> bool {
    app.overlay.as_mut().is_some_and(|o| o.active(Instant::now(), None))
}

/// Drives the viewer through its steps, each after the frames of the one before are presented.
struct Script {
    app: App,
    steps: VecDeque<Step>,
    current: Option<(Step, u64, Instant)>,
    problems: Vec<String>,
}

impl Script {
    fn advance(&mut self, el: &ActiveEventLoop) {
        if self.app.gfx.is_none() {
            return;
        }
        if let Some((step, until, since)) = &self.current {
            let done = step.lasts.map_or(self.app.presented >= *until, |time| since.elapsed() >= time);
            if !done {
                if since.elapsed() > STEP_TIMEOUT {
                    self.problems.push(format!(
                        "{}: only {} frames presented in {:?}",
                        step.name, self.app.presented, STEP_TIMEOUT
                    ));
                    self.steps.clear();
                    self.current = None;
                }
                return;
            }
            if let Some(problem) = (step.expect)(&mut self.app) {
                self.problems.push(format!("{}: {problem}", step.name));
            }
            if let Some(fault) = self.app.gfx.as_ref().and_then(|g| g.canvas.fault()) {
                self.problems.push(format!("{}: drawing stopped: {fault}", step.name));
            }
            self.current = None;
        }
        if let Some(mut next) = self.steps.pop_front() {
            (next.act)(&mut self.app, el);
            if let Some(problem) = (next.at_once)(&mut self.app) {
                self.problems.push(format!("{}: {problem}", next.name));
            }
            self.current = Some((next, self.app.presented + FRAMES_PER_STEP, Instant::now()));
        }
    }

    fn finished(&self) -> bool {
        self.steps.is_empty() && self.current.is_none()
    }
}

impl ApplicationHandler<Event> for Script {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        self.app.resumed(el);
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: Event) {
        self.app.user_event(el, event);
    }

    fn new_events(&mut self, el: &ActiveEventLoop, cause: StartCause) {
        self.app.new_events(el, cause);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        self.app.window_event(el, id, event);
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        self.advance(el);
    }
}

/// Runs the viewer from `launch` through `steps` in a real window; returns every problem seen.
fn run(launch: Launch, steps: Vec<Step>) -> Vec<String> {
    assert!(
        std::env::var_os(DISPLAY_GUARD).is_some(),
        "opens real windows; run it with `just test-display`, which gives it a virtual display"
    );
    let event_loop = EventLoop::<Event>::with_user_event().with_any_thread(true).build().expect("an event loop");
    let app = app(&event_loop, launch, Hooks::default(), Box::new(pollster::block_on));
    let mut script = Script { app, steps: steps.into(), current: None, problems: Vec::new() };
    let mut event_loop = event_loop;
    let deadline = Instant::now() + STEP_TIMEOUT * 4;
    while !script.finished() && Instant::now() < deadline {
        if let PumpStatus::Exit(code) = event_loop.pump_app_events(Some(Duration::from_millis(5)), &mut script) {
            script.problems.push(format!("the viewer exited with {code}"));
            break;
        }
        if let Some(failure) = script.app.failure.take() {
            script.problems.push(format!("the viewer failed: {}", snafu::Report::from_error(failure)));
            break;
        }
    }
    if !script.finished() {
        script.problems.push(format!("ran out of time with {} frames presented", script.app.presented));
    }
    if let Some(g) = script.app.gfx.as_ref() {
        check!(g.failed == 0, "the surface was refusing frames at the end");
    }
    script.problems
}

/// The viewer started from `settings`, drawing even when covered: tests share one display,
/// and another test's full-screen window would otherwise pause this one.
fn launch(mut settings: Settings) -> Launch {
    settings.wallpaper.pause_when_hidden = false;
    Launch {
        settings,
        sources: Sources::embedded(),
        debug: None,
        store: None,
        issues: Vec::new(),
        placement: super::Placement::Window,
        on_window: None,
    }
}

fn first_frames() -> Step {
    step("first frames", |_, _| {}).expect(|app| (app.presented == 0).then(|| "nothing presented".to_owned()))
}

fn menu_is(open: bool) -> impl Fn(&App) -> Option<String> {
    move |app| {
        let shown = app.overlay.as_ref().is_some_and(super::ui::Overlay::menu_open);
        (shown != open).then(|| format!("the menu should be {}", if open { "open" } else { "closed" }))
    }
}

/// The file the menu wrote that froze the viewer at launch.
const FROZE_AT_LAUNCH: &str = "[params]\nobstacle_scale = 0.21\nglare_gain = 0.3\nband_edge_tone = 0.06\nband_width_jitter = -0.05\nspray_mottle = 0.045\n\n[render]\nsupersampling = 1\nadaptive = false\nmax_pixels = 800000.0\nfield_step = 1\nfps_cap = 0\n";

#[derive(Clone, Copy, Debug)]
enum Start {
    Defaults,
    FrozeAtLaunch,
    SmallestCap,
    LargestCap,
    MostSupersampling,
    CoarsestField,
    AdaptiveAtTheFloor,
    SlowestFrameRate,
}

impl Start {
    fn settings(self) -> Settings {
        let mut s = Settings::default();
        let r = &mut s.render;
        match self {
            Self::Defaults => {},
            Self::FrozeAtLaunch => s = toml::load(FROZE_AT_LAUNCH).expect("the file loads").0,
            Self::SmallestCap => r.max_pixels = Some(*limits::MAX_PIXELS.start()),
            Self::LargestCap => r.max_pixels = Some(*limits::MAX_PIXELS.end()),
            Self::MostSupersampling => r.supersampling = *limits::SUPERSAMPLING.end(),
            Self::CoarsestField => r.field_step = *limits::FIELD_STEP.end(),
            Self::AdaptiveAtTheFloor => {
                r.max_pixels = Some(3_686_400.0);
                r.render_scale = *limits::RENDER_SCALE.start();
                r.adaptive = true;
            },
            Self::SlowestFrameRate => r.fps_cap = Some(*limits::FPS_CAP.start()),
        }
        s
    }
}

#[rstest]
#[ignore = "opens windows; run with `just test-display`"]
fn launches_and_keeps_presenting_from_any_saved_settings(
    #[values(
        Start::Defaults,
        Start::FrozeAtLaunch,
        Start::SmallestCap,
        Start::LargestCap,
        Start::MostSupersampling,
        Start::CoarsestField,
        Start::AdaptiveAtTheFloor,
        Start::SlowestFrameRate
    )]
    start: Start,
) {
    let steps = vec![
        first_frames(),
        step("open the menu", |app, el| app.act(el, crate::settings::Action::ToggleMenu)).expect(menu_is(true)),
        step("menu drawn", |_, _| {}).expect(|app| (!app.overlay_live).then(|| "the overlay did not draw".to_owned())),
        step("close the menu", |app, el| app.act(el, crate::settings::Action::ToggleMenu)).expect(menu_is(false)),
    ];
    let problems = run(launch(start.settings()), steps);
    check!(problems.is_empty(), "{problems:#?}");
}

#[test]
#[ignore = "opens windows; run with `just test-display`"]
fn every_menu_change_keeps_frames_coming_with_the_menu_open() {
    let mut steps = vec![first_frames(), step("open the menu", |app, _| app.set_menu(true)).expect(menu_is(true))];
    for (name, change) in extremes() {
        steps.push(step(name, move |app, _| {
            let mut next = app.settings.clone();
            change(&mut next);
            app.change(next);
        }));
    }
    steps.push(step("menu still open", |_, _| {}).expect(menu_is(true)));
    let problems = run(launch(Settings::default()), steps);
    check!(problems.is_empty(), "{problems:#?}");
}

#[test]
#[ignore = "opens windows; run with `just test-display`"]
fn resizing_the_window_with_a_pixel_cap_keeps_frames_coming() {
    let mut settings = Settings::default();
    settings.render.max_pixels = Some(800_000.0);
    let mut steps = vec![first_frames()];
    for size in [(1400, 700), (300, 900), (1, 1), (1600, 1000), (900, 900)] {
        steps.push(step(format!("resize to {size:?}"), move |app, _| {
            if let Some(g) = app.gfx.as_ref() {
                let _ = g.window.request_inner_size(PhysicalSize::new(size.0, size.1));
            }
        }));
    }
    let problems = run(launch(settings), steps);
    check!(problems.is_empty(), "{problems:#?}");
}

#[test]
#[ignore = "opens windows; run with `just test-display`"]
fn keys_open_the_menu_and_hint_at_it() {
    let steps = vec![
        first_frames(),
        step("an unbound key", |app, el| app.key(el, KeyCode::KeyX))
            .at_once(|app| (!overlay_showing(app)).then(|| "no hint showed".to_owned())),
        step("the menu key", |app, el| app.key(el, KeyCode::Escape)).expect(menu_is(true)),
        step("the menu key again", |app, el| app.key(el, KeyCode::Escape)).expect(menu_is(false)),
        step("pause", |app, el| app.key(el, KeyCode::Space))
            .expect(|app| (!app.paused).then(|| "space did not pause".to_owned())),
        step("bind the menu to F1", |app, _| {
            app.command(super::ui::Command::Capture(crate::settings::Action::ToggleMenu));
            app.capture(crate::settings::Action::ToggleMenu, KeyCode::F1);
        })
        .expect(|app| (app.settings.keys.toggle_menu.to_string() != "F1").then(|| "F1 was not bound".to_owned())),
        step("F1 opens the menu", |app, el| app.key(el, KeyCode::F1)).expect(menu_is(true)),
        step("F1 closes it", |app, el| app.key(el, KeyCode::F1)).expect(menu_is(false)),
        step("Esc is now unbound and hints at F1", |app, el| app.key(el, KeyCode::Escape)).at_once(|app| {
            let showing = overlay_showing(app);
            let open = app.overlay.as_ref().is_some_and(super::ui::Overlay::menu_open);
            (!showing || open).then(|| "Esc should hint, not open the menu".to_owned())
        }),
        step("a capture cancelled with Esc keeps the binding", |app, _| {
            app.command(super::ui::Command::Capture(crate::settings::Action::ToggleMenu));
            app.capture(crate::settings::Action::ToggleMenu, KeyCode::Escape);
        })
        .expect(|app| (app.settings.keys.toggle_menu.to_string() != "F1").then(|| "the binding changed".to_owned())),
    ];
    let problems = run(launch(Settings::default()), steps);
    check!(problems.is_empty(), "{problems:#?}");
}

fn pointer(app: &mut App, el: &ActiveEventLoop, event: WindowEvent) {
    let id = app.gfx.as_ref().map(|g| g.window.id()).expect("a window");
    app.window_event(el, id, event);
}

#[test]
#[ignore = "opens windows; run with `just test-display`"]
fn holding_the_right_button_on_the_ball_opens_the_menu() {
    let device_id = DeviceId::dummy();
    let steps = vec![
        first_frames(),
        step("press on the ball", move |app, el| {
            let size = app.gfx.as_ref().map(|g| g.window.inner_size()).expect("a window");
            let view = app.camera.view(crate::camera::Viewport::new(size.width, size.height)).to_array();
            let at = |c: f32, lo: f32, hi: f32, px: u32| f64::from((c - lo) / (hi - lo) * px as f32);
            let x = at(BALL_C[0] + 0.3 * BALL_R, view[0], view[2], size.width);
            let y = at(BALL_C[1], view[1], view[3], size.height);
            pointer(app, el, WindowEvent::CursorMoved { device_id, position: PhysicalPosition::new(x, y) });
            pointer(
                app,
                el,
                WindowEvent::MouseInput { device_id, state: ElementState::Pressed, button: MouseButton::Right },
            );
        })
        .expect(|app| app.opener.progress(Instant::now()).is_none().then(|| "the hold did not start".to_owned())),
        step("hold for a second", |_, _| std::thread::sleep(Duration::from_millis(1100))),
        step("let go", move |app, el| {
            pointer(
                app,
                el,
                WindowEvent::MouseInput { device_id, state: ElementState::Released, button: MouseButton::Right },
            );
        })
        .expect(menu_is(true)),
    ];
    let problems = run(launch(Settings::default()), steps);
    check!(problems.is_empty(), "{problems:#?}");
}

#[test]
#[ignore = "opens windows; run with `just test-display`"]
fn shaders_that_do_not_compile_still_present_the_menu() {
    let mut launch = launch(Settings::default());
    launch.sources.shade = "this is not wgsl".to_owned();
    let steps = vec![
        first_frames().expect(|app| {
            app.gfx.as_ref().and_then(|g| g.canvas.fault()).is_none().then(|| "no fault recorded".to_owned())
        }),
        step("the fault is announced", |_, _| {}).expect(|app| {
            let announced = app.announced_fault.is_some();
            (!announced).then(|| "the fault was never shown to the person".to_owned())
        }),
        step("open the menu", |app, _| app.set_menu(true)).expect(menu_is(true)),
    ];
    let event_loop_problems = run(launch, steps);
    // The steps above expect the fault, so only problems other than it count.
    let problems: Vec<_> = event_loop_problems.into_iter().filter(|p| !p.contains("drawing stopped")).collect();
    check!(problems.is_empty(), "{problems:#?}");
}

fn remote(app: &App) -> Remote {
    Remote(app.proxy.clone())
}

#[test]
#[ignore = "opens windows; run with `just test-display`"]
fn a_remote_changes_settings_and_holds_the_viewer_still() {
    let presented_at_hold = Rc::new(Cell::new(0));
    let at_hold = Rc::clone(&presented_at_hold);
    let steps = vec![
        first_frames(),
        step("set the speed", |app, _| remote(app).set("motion.speed", Some(Value::from(2.5))))
            .expect(|app| (app.settings.motion.speed.total_cmp(&2.5).is_ne()).then(|| "speed not set".to_owned())),
        step("set what is not a setting", |app, _| remote(app).set("render.sparkles", Some(Value::from(true))))
            .expect(|app| (app.settings.motion.speed.total_cmp(&2.5).is_ne()).then(|| "settings moved".to_owned())),
        step("limit the frame rate past its bounds", |app, _| {
            remote(app).set("render.fps_cap", Some(Value::from(9999)));
        })
        .expect(|app| {
            (app.settings.render.fps_cap != Some(*limits::FPS_CAP.end())).then(|| "limit not held".to_owned())
        }),
        step("switch the limit off", |app, _| remote(app).set("render.fps_cap", None))
            .expect(|app| app.settings.render.fps_cap.map(|cap| format!("limit still {cap}"))),
        step("hold", move |app, _| {
            at_hold.set(app.presented);
            remote(app).hold(true);
        })
        .lasting(Duration::from_millis(600))
        .expect(move |app| {
            let since = app.presented - presented_at_hold.get();
            (since > 2).then(|| format!("{since} frames presented while held"))
        }),
        step("release", |app, _| remote(app).hold(false)),
    ];
    let problems = run(launch(Settings::default()), steps);
    check!(problems.is_empty(), "{problems:#?}");
}

/// A mapped top-level window on the test display, standing in for xwinwrap's.
fn parent_window(width: u16, height: u16) -> (x11rb::rust_connection::RustConnection, u32) {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt, CreateWindowAux, WindowClass};

    let (conn, screen) = x11rb::connect(None).expect("the test display");
    let root = conn.setup().roots[screen].root;
    let id = conn.generate_id().expect("a window ID");
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        id,
        root,
        0,
        0,
        width,
        height,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new(),
    )
    .expect("a parent window");
    conn.map_window(id).expect("mapped");
    conn.flush().expect("flushed");
    (conn, id)
}

#[derive(Clone, Copy, Debug)]
enum Place {
    Desktop,
    DesktopByIndex,
    DesktopOnAMissingMonitor,
    Embedded,
}

#[rstest]
#[ignore = "opens windows; run with `just test-display`"]
fn every_wallpaper_placement_keeps_presenting(
    #[values(Place::Desktop, Place::DesktopByIndex, Place::DesktopOnAMissingMonitor, Place::Embedded)] place: Place,
) {
    assert!(std::env::var_os(DISPLAY_GUARD).is_some(), "run it with `just test-display`");
    let mut parent = None;
    let placement = match place {
        Place::Desktop => super::Placement::Desktop { monitor: None },
        Place::DesktopByIndex => super::Placement::Desktop { monitor: Some("0".to_owned()) },
        Place::DesktopOnAMissingMonitor => super::Placement::Desktop { monitor: Some("NOPE-9".to_owned()) },
        Place::Embedded => {
            let (conn, id) = parent_window(480, 270);
            parent = Some(conn);
            super::Placement::Embed(id)
        },
    };
    let desktop = !matches!(place, Place::Embedded);
    let mut launch = launch(Settings::default());
    launch.placement = placement;
    let steps = vec![first_frames().expect(move |app| {
        let size = app.gfx.as_ref()?.window.inner_size();
        // The virtual display is one 1920x1080 monitor, which a desktop window covers.
        (desktop && (size.width, size.height) != (1920, 1080)).then(|| format!("a desktop window sized {size:?}"))
    })];
    let problems = run(launch, steps);
    check!(problems.is_empty(), "{problems:#?}");
    drop(parent);
}

#[test]
#[ignore = "opens windows; run with `just test-display`"]
fn a_fullscreen_program_pauses_the_viewer_unless_the_menu_is_up() {
    let fullscreen = super::Conditions { fullscreen: true, ..super::Conditions::default() };
    let presented_at_pause = Rc::new(Cell::new(0));
    let at_pause = Rc::clone(&presented_at_pause);
    let device_id = DeviceId::dummy();
    let steps = vec![
        first_frames(),
        step("a program goes fullscreen", move |app, _| {
            at_pause.set(app.presented);
            remote(app).conditions(fullscreen);
        })
        .lasting(Duration::from_millis(600))
        .expect(move |app| {
            let since = app.presented - presented_at_pause.get();
            (since > 2).then(|| format!("{since} frames presented under a fullscreen program"))
        }),
        // A click wakes the paused viewer for a frame, and the open menu keeps frames coming.
        step("open the menu", move |app, el| {
            app.set_menu(true);
            pointer(
                app,
                el,
                WindowEvent::MouseInput { device_id, state: ElementState::Pressed, button: MouseButton::Left },
            );
        })
        .expect(menu_is(true)),
        step("close the menu and leave fullscreen", |app, _| {
            app.set_menu(false);
            remote(app).conditions(super::Conditions::default());
        }),
    ];
    let problems = run(launch(Settings::default()), steps);
    check!(problems.is_empty(), "{problems:#?}");
}
