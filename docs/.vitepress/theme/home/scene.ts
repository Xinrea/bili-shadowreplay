import { onBeforeUnmount, reactive, watch, type Ref } from "vue";

/**
 * Plays a scripted demo inside a mock window: a pointer that moves and
 * clicks, typing, and pauses. Each scene describes its steps as one async
 * function; deactivating the scene cancels the running script.
 */

export interface Pointer {
  x: number;
  y: number;
  /** Duration of the move to (x, y), in ms. */
  ms: number;
  down: boolean;
  shown: boolean;
}

export interface Script {
  wait(ms: number): Promise<void>;
  /** Types `text` into a field, one character at a time. */
  type(text: string, set: (value: string) => void, perChar?: number): Promise<void>;
  /** Moves the pointer to the center of an element, or to a point in window coordinates. */
  move(target: Element | null | undefined | [number, number], ms?: number): Promise<void>;
  click(target: Element | null | undefined | [number, number], ms?: number): Promise<void>;
  /** Scrolls a container, such as a dialog body, to `top` or its end. */
  scroll(container: Element | null | undefined, top: number | "end", ms?: number): Promise<void>;
}

class Cancelled extends Error {}

interface SceneOptions {
  active: () => boolean;
  /** The window's content box: pointer coordinates are relative to it, unscaled. */
  root: Ref<HTMLElement | null>;
  reset: () => void;
  run: (script: Script) => Promise<void>;
  /** Called once a run starts, with how long it will take. */
  onStart?: (duration: number) => void;
  onDone?: () => void;
}

const reducedMotion = () =>
  typeof window !== "undefined" &&
  window.matchMedia("(prefers-reduced-motion: reduce)").matches;

export function useScene(options: SceneOptions) {
  const pointer = reactive<Pointer>({ x: 640, y: 520, ms: 0, down: false, shown: false });
  let generation = 0;
  let duration: number | null = null;

  /** Runs the script without waiting, adding up its pauses. */
  async function measure() {
    let total = 0;
    const script: Script = {
      wait: async (ms) => void (total += ms),
      type: async (text, set, perChar = 55) => {
        total += [...text].length * perChar;
        set(text);
      },
      move: async (_target, ms = 650) => void (total += ms),
      click: async (_target, ms = 650) => void (total += ms + 260),
      scroll: async (_container, _top, ms = 500) => void (total += ms),
    };
    await options.run(script);
    return total;
  }

  function pointIn(target: Element | [number, number]): [number, number] {
    if (Array.isArray(target)) return target;
    const root = options.root.value;
    if (!root) return [pointer.x, pointer.y];
    const box = root.getBoundingClientRect();
    const scale = box.width / root.offsetWidth || 1;
    const rect = target.getBoundingClientRect();
    return [
      (rect.left + rect.width / 2 - box.left) / scale,
      (rect.top + rect.height / 2 - box.top) / scale,
    ];
  }

  function live(id: number): Script {
    const check = () => {
      if (id !== generation) throw new Cancelled();
    };
    const wait = (ms: number) =>
      new Promise<void>((resolve, reject) => {
        setTimeout(() => (id === generation ? resolve() : reject(new Cancelled())), ms);
      });
    const move = async (target: Parameters<Script["move"]>[0], ms = 650) => {
      check();
      if (!target) return wait(ms);
      const [x, y] = pointIn(target);
      pointer.shown = true;
      pointer.ms = ms;
      pointer.x = x;
      pointer.y = y;
      await wait(ms);
    };
    return {
      wait,
      move,
      type: async (text, set, perChar = 55) => {
        const chars = [...text];
        for (let i = 1; i <= chars.length; i++) {
          check();
          set(chars.slice(0, i).join(""));
          await wait(perChar);
        }
      },
      click: async (target, ms = 650) => {
        await move(target, ms);
        pointer.down = true;
        await wait(140);
        pointer.down = false;
        await wait(120);
      },
      scroll: async (container, top, ms = 500) => {
        check();
        container?.scrollTo({
          top: top === "end" ? container.scrollHeight : top,
          behavior: "smooth",
        });
        await wait(ms);
      },
    };
  }

  async function start() {
    const id = ++generation;
    if (duration === null) {
      duration = await measure();
      if (id !== generation) return;
    }
    options.reset();
    pointer.shown = false;
    pointer.ms = 0;
    if (reducedMotion()) {
      // Show the finished state instead of playing it.
      await measure();
      options.onStart?.(0);
      options.onDone?.();
      return;
    }
    options.onStart?.(duration);
    try {
      await options.run(live(id));
      if (id === generation) options.onDone?.();
    } catch (error) {
      if (!(error instanceof Cancelled)) throw error;
    }
  }

  watch(
    options.active,
    (active) => {
      if (active) void start();
      else generation++;
    },
    { immediate: true, flush: "post" },
  );

  onBeforeUnmount(() => generation++);

  return { pointer, replay: start };
}
