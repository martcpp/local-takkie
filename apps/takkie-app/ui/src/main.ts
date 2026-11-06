import { mount } from 'svelte'
import '@fontsource-variable/archivo/wdth.css'
import '@fontsource-variable/jetbrains-mono/wght.css'
import './app.css'
import App from './App.svelte'

const app = mount(App, {
  target: document.getElementById('app')!,
})

export default app
