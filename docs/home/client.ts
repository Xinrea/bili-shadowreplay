import { createSSRApp } from 'vue';
import HomePage from './HomePage.vue';

export function mount(element: HTMLElement) {
  const app = createSSRApp(HomePage);
  app.mount(element);
  return () => app.unmount();
}
