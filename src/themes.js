(function () {
  const ACCENTS = {
    amber: '#F59E0B',
    blue: '#3B82F6',
    green: '#34D399',
    purple: '#A78BFA',
    cyan: '#22D3EE',
    rose: '#FB7185',
  };

  const clamp = (value, min, max) => Math.min(max, Math.max(min, value));

  function normalizeHex(value) {
    let hex = String(value || '').trim().replace(/^#/, '');
    if (/^[0-9a-f]{3}$/i.test(hex)) hex = hex.split('').map(char => char + char).join('');
    return /^[0-9a-f]{6}$/i.test(hex) ? `#${hex.toUpperCase()}` : null;
  }

  function hexToRgb(hex) {
    const clean = normalizeHex(hex).slice(1);
    return {
      r: parseInt(clean.slice(0, 2), 16),
      g: parseInt(clean.slice(2, 4), 16),
      b: parseInt(clean.slice(4, 6), 16),
    };
  }

  function rgbToHex(r, g, b) {
    const channel = value => clamp(Math.round(value), 0, 255).toString(16).padStart(2, '0');
    return `#${channel(r)}${channel(g)}${channel(b)}`.toUpperCase();
  }

  function hexToHsv(hex) {
    let { r, g, b } = hexToRgb(hex);
    r /= 255; g /= 255; b /= 255;
    const max = Math.max(r, g, b), min = Math.min(r, g, b), delta = max - min;
    let h = 0;
    if (delta) {
      if (max === r) h = 60 * (((g - b) / delta) % 6);
      else if (max === g) h = 60 * ((b - r) / delta + 2);
      else h = 60 * ((r - g) / delta + 4);
    }
    if (h < 0) h += 360;
    return { h: Math.round(h), s: Math.round(max ? delta / max * 100 : 0), v: Math.round(max * 100) };
  }

  function hsvToHex(h, s, v) {
    h = ((Number(h) % 360) + 360) % 360;
    s = clamp(Number(s), 0, 100) / 100;
    v = clamp(Number(v), 0, 100) / 100;
    const c = v * s, x = c * (1 - Math.abs((h / 60) % 2 - 1)), m = v - c;
    let rgb = [0, 0, 0];
    if (h < 60) rgb = [c, x, 0];
    else if (h < 120) rgb = [x, c, 0];
    else if (h < 180) rgb = [0, c, x];
    else if (h < 240) rgb = [0, x, c];
    else if (h < 300) rgb = [x, 0, c];
    else rgb = [c, 0, x];
    return rgbToHex((rgb[0] + m) * 255, (rgb[1] + m) * 255, (rgb[2] + m) * 255);
  }

  function applyAccent(hex) {
    const normalized = normalizeHex(hex) || ACCENTS.cyan;
    const { r, g, b } = hexToRgb(normalized);
    const style = document.documentElement.style;
    style.setProperty('--accent', normalized);
    style.setProperty('--accent-2', rgbToHex(r * .72, g * .72, b * .72));
    style.setProperty('--accent-rgb', `${r}, ${g}, ${b}`);
    style.setProperty('--accent-bg', `rgba(${r}, ${g}, ${b}, .14)`);
    style.setProperty('--accent-bd', `rgba(${r}, ${g}, ${b}, .38)`);
    const mix = (base, amount) => rgbToHex(
      base[0] * (1 - amount) + r * amount,
      base[1] * (1 - amount) + g * amount,
      base[2] * (1 - amount) + b * amount,
    );
    style.setProperty('--accent-dark-bg', mix([10, 12, 16], .08));
    style.setProperty('--accent-dark-surface', mix([17, 20, 26], .09));
    style.setProperty('--accent-dark-surface-2', mix([25, 29, 37], .10));
    style.setProperty('--accent-dark-surface-3', mix([34, 39, 49], .12));
    return normalized;
  }

  function applyUiTheme(name) {
    const theme = ['modern', 'studio'].includes(name) ? name : 'studio';
    document.documentElement.dataset.uiTheme = theme;
    document.querySelectorAll('.theme-card').forEach(card => card.classList.toggle('active', card.dataset.uiTheme === theme));
    return theme;
  }

  window.EffigyThemes = { ACCENTS, normalizeHex, hexToHsv, hsvToHex, applyAccent, applyUiTheme };
})();
