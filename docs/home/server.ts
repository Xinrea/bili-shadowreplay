import { createSSRApp } from 'vue';
import { renderToString } from '@vue/server-renderer';
import HomePage from './HomePage.vue';

export function render() {
  return renderToString(createSSRApp(HomePage));
}
