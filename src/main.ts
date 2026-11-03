import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import { loadConnections } from './lib/state.svelte';

loadConnections();

export default mount(App, { target: document.getElementById('app')! });
