import "./styles.css";
import { mount } from "svelte";
import App from "./AppLive.svelte";

const app = mount(App as any, {
  target: document.getElementById("app")!,
});

export default app;
