/**
 * Listens for Wallpaper Engine and Lively, which call these globals from the moment the page
 * loads and send each property once. The bridge keeps the latest of everything they sent and
 * hands it all to each viewer that starts, since a viewer started again after losing its
 * device would otherwise come up without the person's settings. Also passes on the battery.
 */
export function hostBridge(log) {
  const properties = new Map();
  let held = false;
  let onBattery = null;
  let viewer = null;

  function apply(target, call, what) {
    try {
      call(target);
    } catch (error) {
      log("host-rejected", { what, message: error?.message ?? String(error) });
    }
  }

  function property(key, value) {
    properties.set(key, value);
    if (viewer) apply(viewer, (v) => v.host(key, value), key);
  }

  function hold(value) {
    held = Boolean(value);
    if (viewer) apply(viewer, (v) => v.hold(held), "hold");
  }

  window.wallpaperPropertyListener = {
    applyUserProperties(changed) {
      for (const [key, entry] of Object.entries(changed)) property(key, entry.value);
    },
    applyGeneralProperties(changed) {
      if ("fps" in changed) property("fps", changed.fps);
    },
    setPaused: hold,
  };
  window.livelyPropertyListener = property;
  window.livelyWallpaperPlaybackChanged = (data) => {
    try {
      hold(JSON.parse(data).IsPaused);
    } catch (error) {
      log("host-rejected", { what: "playback", message: error?.message ?? String(error) });
    }
  };

  navigator.getBattery?.().then(
    (battery) => {
      const tell = () => {
        onBattery = !battery.charging;
        if (viewer) apply(viewer, (v) => v.power(onBattery), "power");
      };
      tell();
      battery.addEventListener("chargingchange", tell);
    },
    (error) => log("no-battery", { message: error?.message ?? String(error) }),
  );

  return {
    /** Hands `next` everything the host has said so far, and everything it says from now on. */
    attach(next) {
      viewer = next;
      if (!viewer) return;
      for (const [key, value] of properties) apply(viewer, (v) => v.host(key, value), key);
      if (onBattery !== null) apply(viewer, (v) => v.power(onBattery), "power");
      if (held) apply(viewer, (v) => v.hold(true), "hold");
      log("host-attached", { properties: properties.size, held, onBattery });
    },
  };
}
