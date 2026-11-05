import { createApp } from 'vue'
import App from './App.vue'
import { vuetify } from './plugins/vuetify'
import 'vuetify/styles'
import '@mdi/font/css/materialdesignicons.css'
import './style.css'

createApp(App).use(vuetify).mount('#app')
