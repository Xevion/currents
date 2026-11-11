/** Workarounds for browsers that differ from the specs the viewer relies on. */

/**
 * WebKit fires `uncapturederror` only for listeners added with `addEventListener`, while wgpu
 * assigns `onuncapturederror`; without this, device errors on Safari would go unseen.
 */
export function shimUncapturedError() {
  const proto = globalThis.GPUDevice?.prototype;
  if (!proto || "onuncapturederror" in proto) return false;
  const handlers = new WeakMap();
  Object.defineProperty(proto, "onuncapturederror", {
    configurable: true,
    get() {
      return handlers.get(this) ?? null;
    },
    set(handler) {
      const old = handlers.get(this);
      if (old) this.removeEventListener("uncapturederror", old);
      if (typeof handler === "function") {
        handlers.set(this, handler);
        this.addEventListener("uncapturederror", handler);
      } else {
        handlers.delete(this);
      }
    },
  });
  return true;
}

/**
 * The viewer sizes the canvas from `devicePixelContentBoxSize` but scales input and the menu by
 * `devicePixelRatio`. Where the two disagree (DevTools device emulation reports host pixels),
 * hiding the former makes it size from CSS pixels times the ratio, so every measure agrees.
 */
export function agreeOnPixelSize(element) {
  if (!("devicePixelContentBoxSize" in ResizeObserverEntry.prototype)) return Promise.resolve(true);
  return new Promise((resolve) => {
    const observer = new ResizeObserver(([entry]) => {
      observer.disconnect();
      const device = entry.devicePixelContentBoxSize[0].inlineSize;
      const css = entry.contentRect.width * devicePixelRatio;
      const agree = Math.abs(device - css) <= 2;
      if (!agree) delete ResizeObserverEntry.prototype.devicePixelContentBoxSize;
      resolve(agree);
    });
    observer.observe(element, { box: "device-pixel-content-box" });
  });
}
