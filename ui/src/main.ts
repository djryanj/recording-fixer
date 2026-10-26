import './app.css';
import { mount } from 'svelte';
import App from './App.svelte';
import { installErrorLogging } from './lib/diagnostics';

installErrorLogging();

export default mount(App, { target: document.getElementById('app')! });
