/**
 * IPTVMine Desktop Showcase Switcher
 * Handles tab transitions and cross-fading screenshots
 */
function switchScreenshot(src, btn) {
  const img = document.getElementById('showcase-img');
  if (!img) return;

  img.style.opacity = '0';
  setTimeout(() => {
    img.src = src;
    img.style.opacity = '1';
  }, 150);

  document.querySelectorAll('.showcase-tab').forEach((t) => t.classList.remove('active'));
  if (btn) {
    btn.classList.add('active');
  }
}
