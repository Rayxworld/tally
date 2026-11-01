const screenshot = document.querySelector('.app-screenshot')
const caption = document.querySelector('#preview-caption')
const themeButtons = document.querySelectorAll('.theme-button')

for (const button of themeButtons) {
  button.addEventListener('click', () => {
    const theme = button.dataset.theme
    const isDark = theme === 'dark'
    screenshot.classList.add('is-changing')
    screenshot.src = isDark ? screenshot.dataset.dark : screenshot.dataset.light
    screenshot.alt = `Tally desktop app browsing folders in ${theme} mode`
    caption.textContent = isDark
      ? 'A little easier on the eyes after hours.'
      : 'A little more light for the daytime.'

    for (const option of themeButtons) {
      const selected = option === button
      option.classList.toggle('is-active', selected)
      option.setAttribute('aria-pressed', String(selected))
    }

    screenshot.addEventListener('load', () => screenshot.classList.remove('is-changing'), { once: true })
  })
}
