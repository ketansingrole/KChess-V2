import { boardFit } from '../utils/boardFit'

export default defineNuxtPlugin((nuxtApp) => {
  nuxtApp.vueApp.directive('board-fit', boardFit)
})
