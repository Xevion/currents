// Listens for Wallpaper Engine and Lively, which call these globals from the moment the page
// loads, often before the viewer is up; calls wait until it is. On any other page they are
// never called.
(() => {
  const waiting = [];
  const send = (call) => (window.currents ? call(window.currents) : waiting.push(call));
  document.addEventListener("currents:ready", () => waiting.splice(0).forEach((call) => call(window.currents)));

  const property = (key, value) =>
    send((viewer) => {
      try {
        viewer.host(key, value);
      } catch (error) {
        console.warn(`currents: ${error.message ?? error}`);
      }
    });
  const hold = (held) => send((viewer) => viewer.hold(Boolean(held)));

  window.wallpaperPropertyListener = {
    applyUserProperties(properties) {
      for (const [key, property_] of Object.entries(properties)) property(key, property_.value);
    },
    applyGeneralProperties(properties) {
      if ("fps" in properties) property("fps", properties.fps);
    },
    setPaused: hold,
  };

  window.livelyPropertyListener = property;
  window.livelyWallpaperPlaybackChanged = (data) => {
    try {
      hold(JSON.parse(data).IsPaused);
    } catch (error) {
      console.warn("currents: unreadable playback state", error);
    }
  };
})();
