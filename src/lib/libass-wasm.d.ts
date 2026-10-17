declare module "libass-wasm" {
  export default class SubtitlesOctopus {
    constructor(options: {
      canvas: HTMLCanvasElement;
      subContent: string;
      workerUrl: string;
      fonts?: string[];
      fallbackFont?: string;
      availableFonts?: Record<string, string>;
      renderMode?: "wasm-blend" | "js-blend";
      targetFps?: number;
      libassMemoryLimit?: number;
      libassGlyphLimit?: number;
      onReady?: () => void;
      onError?: (error: unknown) => void;
    });
    worker: Worker | null;
    setTrack(content: string): void;
    setCurrentTime(time: number): void;
    setIsPaused(paused: boolean, time: number): void;
    setRate(rate: number): void;
    dispose(): void;
  }
}
