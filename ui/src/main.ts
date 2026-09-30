import { mount } from 'svelte';
import { invoke } from '@tauri-apps/api/core';
import './app.css';
import App from './App.svelte';

// Uncaught errors go to the launcher's output, where they can be read without developer tools.
const report = (text: string) => invoke('report_error', { text }).catch(() => {});
window.addEventListener('error', (e) => report(`${e.message} at ${e.filename}:${e.lineno}`));
window.addEventListener('unhandledrejection', (e) => report(`Unhandled: ${e.reason?.stack ?? e.reason}`));

export default mount(App, { target: document.getElementById('app')! });
