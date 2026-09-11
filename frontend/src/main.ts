/// <reference types="vite/client" />
import { mount } from 'svelte';
import App from './App.svelte';
import './app.css';

const target = document.getElementById('app');
if (!target) {
  throw new Error('Lorehaven could not find its mount point (#app).');
}

export default mount(App, { target });

/**
 * Register the offline worker (spec §13.5).
 *
 * Production only. In development the worker would cache the modules being
 * edited, so a change would appear not to take effect — and the failure mode is
 * invisible, because the page still works, just from yesterday's code.
 *
 * A failure to register is not reported: an instance served over plain HTTP on a
 * LAN cannot register a worker, and that instance's readers lose offline reading
 * and nothing else.
 */
if (import.meta.env.PROD && 'serviceWorker' in navigator) {
  window.addEventListener('load', () => {
    void navigator.serviceWorker.register('/service-worker.js').catch(() => undefined);
  });
}
