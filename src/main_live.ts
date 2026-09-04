import "./styles.css";
import { mount } from "svelte";
import App from "./AppLive.svelte";

const app = mount(App, {
  target: document.getElementById("app")!,
});

export default app;
