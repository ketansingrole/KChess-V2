/** Shared control heights include the segmented control's track and border. */
const controlSizes = {
  xs: { base: 'min-h-6' },
  sm: { base: 'min-h-7' },
  md: { base: 'min-h-8' },
  lg: { base: 'min-h-9' },
  xl: { base: 'min-h-10' },
}

export default defineAppConfig({
  ui: {
    button: { variants: { size: controlSizes } },
    input: { variants: { size: controlSizes } },
    select: {
      variants: { size: controlSizes },
      slots: {
        content:
          'w-max min-w-(--reka-select-trigger-width) max-w-[min(28rem,var(--reka-select-content-available-width,calc(100vw-2rem)))]',
        itemLabel: 'whitespace-normal break-words',
        itemDescription: 'whitespace-normal break-words',
      },
    },
    tabs: {
      slots: { list: 'p-0.5' },
      variants: {
        size: {
          xs: { list: 'min-h-6', trigger: 'min-h-[18px] py-0' },
          sm: { list: 'min-h-7', trigger: 'min-h-[22px] py-0' },
          md: { list: 'min-h-8', trigger: 'min-h-[26px] py-0' },
          lg: { list: 'min-h-9', trigger: 'min-h-[30px] py-0' },
          xl: { list: 'min-h-10', trigger: 'min-h-[34px] py-0' },
        },
      },
    },
  },
})
