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
          'shadow-none w-max min-w-(--reka-select-trigger-width) max-w-[min(28rem,var(--reka-select-content-available-width,calc(100vw-2rem)))]',
        itemLabel: 'whitespace-normal break-words',
        itemDescription: 'whitespace-normal break-words',
      },
    },
    // Flat surfaces belong to component slots; focus rings remain intact.
    modal: { variants: { fullscreen: { false: { content: 'shadow-none' } } } },
    toast: { slots: { root: 'shadow-none' } },
    tooltip: { slots: { content: 'shadow-none' } },
    popover: { slots: { content: 'shadow-none' } },
    dropdownMenu: { slots: { content: 'shadow-none' } },
    contextMenu: { slots: { content: 'shadow-none' } },
    selectMenu: { slots: { content: 'shadow-none' } },
    inputMenu: { slots: { content: 'shadow-none' } },
    slideover: { slots: { content: 'sm:shadow-none' } },
    switch: { slots: { thumb: 'shadow-none' } },
    navigationMenu: {
      slots: { viewport: 'shadow-none' },
      compoundVariants: [
        { orientation: 'vertical', collapsed: true, class: { content: 'shadow-none' } },
      ],
    },
    sidebar: { variants: { variant: { floating: { inner: 'shadow-none' } } } },
    tabs: {
      slots: { list: 'p-0.5' },
      compoundVariants: [
        {
          variant: 'pill',
          orientation: 'horizontal',
          class: {
            list: 'flex-wrap h-auto bg-default border border-default',
            indicator: 'hidden',
            trigger: 'h-auto',
            label: 'whitespace-normal overflow-visible text-clip text-center leading-[1.25]',
          },
        },
        {
          variant: 'pill',
          orientation: 'horizontal',
          color: 'primary',
          class: { trigger: 'data-[state=active]:bg-primary' },
        },
        {
          variant: 'pill',
          orientation: 'horizontal',
          color: 'secondary',
          class: { trigger: 'data-[state=active]:bg-secondary' },
        },
        {
          variant: 'pill',
          orientation: 'horizontal',
          color: 'success',
          class: { trigger: 'data-[state=active]:bg-success' },
        },
        {
          variant: 'pill',
          orientation: 'horizontal',
          color: 'info',
          class: { trigger: 'data-[state=active]:bg-info' },
        },
        {
          variant: 'pill',
          orientation: 'horizontal',
          color: 'warning',
          class: { trigger: 'data-[state=active]:bg-warning' },
        },
        {
          variant: 'pill',
          orientation: 'horizontal',
          color: 'error',
          class: { trigger: 'data-[state=active]:bg-error' },
        },
        {
          variant: 'pill',
          orientation: 'horizontal',
          color: 'neutral',
          class: { trigger: 'data-[state=active]:bg-inverted' },
        },
      ],
      variants: {
        variant: { pill: { indicator: 'shadow-none' } },
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
