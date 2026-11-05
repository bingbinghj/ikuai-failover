import { createVuetify } from 'vuetify'
import * as components from 'vuetify/components'
import * as directives from 'vuetify/directives'
import { aliases, mdi } from 'vuetify/iconsets/mdi'
import { zhHans } from 'vuetify/locale'
import { getMoviePilotThemePalette, initialColorScheme } from '../theme'

export const vuetify = createVuetify({
  components,
  directives,
  aliases: { IconBtn: components.VBtn },
  icons: { defaultSet: 'mdi', aliases, sets: { mdi } },
  locale: { locale: 'zhHans', fallback: 'en', messages: { zhHans } },
  theme: {
    defaultTheme: initialColorScheme(),
    themes: { light: getMoviePilotThemePalette('light'), dark: getMoviePilotThemePalette('dark') },
  },
  defaults: {
    IconBtn: { icon: true, color: 'default', variant: 'text', VIcon: { size: 24 } },
    VAlert: { VBtn: { color: undefined } },
    VAvatar: { variant: 'flat', VIcon: { size: 24 } },
    VBadge: { color: 'primary' },
    VBtn: { color: 'primary', elevation: 0 },
    VCard: { elevation: 0, rounded: 'lg' },
    VMenu: { elevation: 0, transition: 'mp-popover-transition' },
    VChip: { elevation: 0 },
    VDialog: { elevation: 0, rounded: 'lg', transition: 'mp-dialog-transition' },
    VExpansionPanels: { elevation: 0 },
    VList: { color: 'primary', elevation: 0 },
    VListItem: { rounded: 'md' },
    VPagination: { activeColor: 'primary' },
    VTabs: { color: 'primary', VSlideGroup: { showArrows: true } },
    VTooltip: { location: 'top', transition: 'mp-popover-transition' },
    VCheckboxBtn: { color: 'primary', hideDetails: 'auto' },
    VCheckbox: { color: 'primary', hideDetails: 'auto' },
    VRadioGroup: { color: 'primary', hideDetails: 'auto' },
    VRadio: { color: 'primary', hideDetails: 'auto' },
    VSelect: { variant: 'outlined', color: 'primary', hideDetails: 'auto', menuProps: { elevation: 0 } },
    VRangeSlider: { color: 'primary', density: 'comfortable', thumbLabel: true, hideDetails: 'auto' },
    VProgressCircular: { color: 'primary' },
    VSlider: { color: 'primary', hideDetails: 'auto' },
    VTextField: { variant: 'outlined', color: 'primary', hideDetails: 'auto' },
    VAutocomplete: { variant: 'outlined', color: 'primary', hideDetails: 'auto' },
    VCombobox: { variant: 'outlined', color: 'primary', hideDetails: 'auto', menuProps: { elevation: 0 } },
    VFileInput: { variant: 'outlined', color: 'primary', hideDetails: 'auto' },
    VTextarea: { variant: 'outlined', color: 'primary', hideDetails: 'auto' },
    VSwitch: { color: 'primary', hideDetails: 'auto' },
  },
})
