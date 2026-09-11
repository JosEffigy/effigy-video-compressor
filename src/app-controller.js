// Application controller, initialized after the Svelte shell mounts.
// ── Resolution-aware scaling ──────────────────────────────────────────────────
(function scaleToScreen() {
  const physW = window.screen.width * (window.devicePixelRatio || 1);
  const base  = physW >= 3840 ? 18 : physW >= 2560 ? 16 : physW >= 1920 ? 15 : 14;
  document.documentElement.style.fontSize = base + 'px';
})();

// ── Tauri bridge ──────────────────────────────────────────────────────────────
const desktopBridge = window.__TAURI__;
const invoke = desktopBridge?.core?.invoke || (async command => {
  if (command === 'load_settings') return null;
  if (command === 'check_ffmpeg') return { ffmpeg_ok:true, ffprobe_ok:true, ffmpeg_version:'ffmpeg version preview', source:'preview' };
  if (command === 'detect_hardware_encoders' || command === 'select_files') return [];
  return null;
});
const listen = desktopBridge?.event?.listen || (async () => () => {});

// ── Themes ────────────────────────────────────────────────────────────────────
const Theme = window.EffigyThemes;

// ── App state ─────────────────────────────────────────────────────────────────
// Each file: { path, name, status, detail, thumbUrl }
// status: 'queued' | 'active' | 'done' | 'error'
const state = {
  files: [], running: false,
  codec:'libx264', codec_name:'x264', crf:24, quality_mode:'crf', gpu_gameplay_mode:'native',
  container:'mp4', algorithm:'dynamic',
  res_mode:'auto', res_w:null, res_h:null, resolution_name:'auto',
  fps:0, fps_name:'auto',
  target_size:8, size_name:'8mb', allocation_mode:'lightweight',
  preset:'slower', svt_preset:6,
};

const prefs = {
  uiTheme:'studio', accent:'rose', customAccent:'#22D3EE',
  mode:'dark', openFolder:false, rememberSettings:true,
  startOnAdd:false, minimizeToTray:false, outputFolder:'', persistOutputFolder:false,
};

// ── DOM refs ──────────────────────────────────────────────────────────────────
const $ = id => document.getElementById(id);
const queue         = $('queue');
const compressBtn   = $('compress-btn');
const summary       = $('summary');
const ffmpegDot     = $('ffmpeg-dot');
const ffmpegLabel   = $('ffmpeg-label');
const ffmpegStatusBtn = $('ffmpeg-status-btn');
const ffmpegMenu      = $('ffmpeg-menu');
const ffmpegAlert     = $('ffmpeg-alert');
const logOutput     = $('log-output');
const settingsPanel = $('settings-panel');
const backdrop      = $('settings-backdrop');
const ffmpegWarning = $('ffmpeg-warning');
const dropZone      = $('drop-zone');
const dropSub       = dropZone.querySelector('.drop-sub');
const DROP_SUB_DEFAULT = dropSub.textContent;

// ── Theme / mode ──────────────────────────────────────────────────────────────
function applyTheme(name) {
  const isCustom = name === 'custom';
  const color = isCustom ? prefs.customAccent : (Theme.ACCENTS[name] || Theme.ACCENTS.rose);
  prefs.accent = isCustom ? 'custom' : (Theme.ACCENTS[name] ? name : 'rose');
  prefs.customAccent = isCustom ? Theme.applyAccent(color) : prefs.customAccent;
  Theme.applyAccent(color);
  document.querySelectorAll('.swatch').forEach(sw => sw.classList.toggle('active', sw.dataset.theme === name));
  $('custom-color-panel').hidden = !isCustom;
  if (isCustom) syncCustomColorControls(prefs.customAccent);
  savePrefs();
}

function applyUiTheme(name) {
  prefs.uiTheme = Theme.applyUiTheme(name);
  document.querySelectorAll('.option-group.studio-open').forEach(group => group.classList.remove('studio-open'));
  savePrefs();
}

const systemDark = window.matchMedia('(prefers-color-scheme: dark)');
function applyMode(mode) {
  prefs.mode = mode;
  const isLight = mode === 'light' || (mode === 'system' && !systemDark.matches);
  document.documentElement.classList.toggle('light', isLight);
  document.querySelectorAll('.mode-btn[data-mode]').forEach(b => b.classList.toggle('active', b.dataset.mode === mode));
  savePrefs();
}
systemDark.addEventListener('change', () => { if (prefs.mode === 'system') applyMode('system'); });
document.querySelectorAll('.mode-btn[data-mode]').forEach(b => b.addEventListener('click', () => applyMode(b.dataset.mode)));

// ── Portable .cfg persistence ─────────────────────────────────────────────────
const LEGACY_LS = 'effigy-v1';
let settingsReady = false;
let settingsSaveTimer = 0;

function serializedSettings() {
  return {
    version: 7,
    codec:state.codec, codec_name:state.codec_name, crf:state.crf,
    container:state.container, algorithm:state.algorithm,
    res_mode:state.res_mode, res_w:state.res_w, res_h:state.res_h,
    resolution_name:state.resolution_name,
    fps:state.fps, fps_name:state.fps_name,
    target_size:state.target_size, size_name:state.size_name, allocation_mode:state.allocation_mode,
    preset:state.preset, svt_preset:state.svt_preset,
    uiTheme:prefs.uiTheme, accent:prefs.accent, customAccent:prefs.customAccent, mode:prefs.mode,
    openFolder:prefs.openFolder, rememberSettings:prefs.rememberSettings,
    startOnAdd:prefs.startOnAdd, minimizeToTray:prefs.minimizeToTray,
    persistOutputFolder:prefs.persistOutputFolder,
    outputFolder:prefs.persistOutputFolder ? prefs.outputFolder : null,
  };
}

function savePrefs() {
  if (!settingsReady) return;
  clearTimeout(settingsSaveTimer);
  settingsSaveTimer = setTimeout(() => {
    invoke('save_settings', { settings: serializedSettings() })
      .catch(error => addLog(`Could not save effigy-video-compressor.cfg: ${error}`, 'err'));
  }, 120);
}

function mergePrefs(s) {
  if (!s) return;
  if (s.rememberSettings != null) prefs.rememberSettings = s.rememberSettings;
  const pick = key => { if (s[key] != null) state[key] = s[key]; };
  if (prefs.rememberSettings) {
    ['codec','codec_name','container','algorithm','res_mode','res_w','res_h','resolution_name',
     'fps','fps_name','target_size','size_name','allocation_mode','preset','svt_preset'].forEach(pick);
    // Version 1 stored the visible quality control as `cqp`; its separate
    // `crf` value was unused. Preserve what the user actually selected.
    if ((Number(s.version) || 0) < 5 && s.cqp != null) state.crf = s.cqp;
    else if (s.crf != null) state.crf = s.crf;
  }
  if (s.uiTheme) prefs.uiTheme = s.uiTheme;
  if (s.accent) prefs.accent = s.accent;
  else if (s.theme) prefs.accent = s.theme;
  if (Theme.normalizeHex(s.customAccent)) prefs.customAccent = Theme.normalizeHex(s.customAccent);
  if (s.mode) prefs.mode = s.mode;
  else if (s.lightMode != null) prefs.mode = s.lightMode ? 'light' : 'dark';
  if (s.openFolder != null) prefs.openFolder = s.openFolder;
  if (s.startOnAdd != null) prefs.startOnAdd = s.startOnAdd;
  if (s.minimizeToTray != null) prefs.minimizeToTray = s.minimizeToTray;
  prefs.persistOutputFolder = s.persistOutputFolder === true;
  prefs.outputFolder = prefs.persistOutputFolder && typeof s.outputFolder === 'string' ? s.outputFolder : '';
}

async function loadPrefs() {
  let loaded = null;
  try {
    loaded = await invoke('load_settings');
  } catch (error) {
    addLog(`Could not read effigy-video-compressor.cfg; using defaults: ${error}`, 'err');
  }
  if (!loaded) {
    try { loaded = JSON.parse(localStorage.getItem(LEGACY_LS) || 'null'); } catch {}
  }
  mergePrefs(loaded);
  localStorage.removeItem(LEGACY_LS);
  settingsReady = true;
}
function applyLoadedPrefs() {
  applyUiTheme(prefs.uiTheme); applyTheme(prefs.accent); applyMode(prefs.mode);
  setToggle('toggle-open-folder', prefs.openFolder);
  setToggle('toggle-remember',    prefs.rememberSettings);
  setToggle('toggle-start-on-add', prefs.startOnAdd);
  setToggle('toggle-minimize-tray', prefs.minimizeToTray);
  invoke('set_minimize_to_tray', { enabled:prefs.minimizeToTray });
  $('persist-output-folder').checked = prefs.persistOutputFolder;
  syncOutputFolderUi();
  activateGroup('codec',  state.codec_name);
  activateGroup('container', state.container);
  activateGroup('algorithm', state.algorithm);
  activateGroup('res',    state.resolution_name);
  activateGroup('fps',    state.fps_name === 'auto' ? 'auto' : state.fps === 60 ? '60' : state.fps === 30 ? '30' : 'custom');
  activateGroup('size',   [8,10,20,25].includes(state.target_size) ? String(state.target_size) : 'custom');
  activateGroup('allocation', state.allocation_mode);
  activateGroup('preset', state.preset);
  setQuality(state.crf);
  syncAlgorithmUi();
  syncContainerUi();
  syncPresetUi();
}
function activateGroup(group, value) {
  document.querySelectorAll(`[data-group="${group}"]`)
          .forEach(b => b.classList.toggle('active', b.dataset.value === value));
}

// ── Toggle ────────────────────────────────────────────────────────────────────
function setToggle(id, active) { const el=$(id); if(el) el.dataset.active=String(!!active); }
function initToggle(id, onChange) {
  const el = $(id); if (!el) return;
  el.addEventListener('click', e => { e.stopPropagation(); const v=el.dataset.active!=='true'; el.dataset.active=String(v); onChange(v); });
  el.closest('.toggle-row')?.addEventListener('click', e => { if(el.contains(e.target)) return; el.click(); });
}

// ── Settings panel ────────────────────────────────────────────────────────────
$('settings-btn').addEventListener('click', () => { settingsPanel.classList.add('open'); backdrop.classList.add('visible'); $('settings-btn').classList.add('active'); });
function closeSettings() { settingsPanel.classList.remove('open'); backdrop.classList.remove('visible'); $('settings-btn').classList.remove('active'); }
$('close-settings-btn').addEventListener('click', closeSettings);
backdrop.addEventListener('click', closeSettings);
document.querySelectorAll('.swatch').forEach(s => s.addEventListener('click', () => applyTheme(s.dataset.theme)));
document.querySelectorAll('.theme-card').forEach(card => card.addEventListener('click', () => applyUiTheme(card.dataset.uiTheme)));
document.querySelectorAll('.option-group').forEach(group => group.addEventListener('click', event => {
  if (document.documentElement.dataset.uiTheme !== 'studio' || !event.target.closest('.option-btn')) return;
  const clickedActive = event.target.closest('.option-btn').classList.contains('active');
  document.querySelectorAll('.option-group.studio-open').forEach(openGroup => {
    if (openGroup !== group) openGroup.classList.remove('studio-open');
  });
  if (group.classList.contains('studio-open') && !clickedActive) group.classList.remove('studio-open');
  else group.classList.toggle('studio-open');
}));
document.addEventListener('click', event => {
  if (event.target.closest('.option-group')) return;
  document.querySelectorAll('.option-group.studio-open').forEach(group => group.classList.remove('studio-open'));
});
document.addEventListener('keydown', event => {
  if (event.key !== 'Escape') return;
  document.querySelectorAll('.option-group.studio-open').forEach(group => group.classList.remove('studio-open'));
});

const accentHex = $('accent-hex');
const accentH = $('accent-h');
const accentS = $('accent-s');
const accentV = $('accent-v');

function syncCustomColorControls(hex) {
  const normalized = Theme.normalizeHex(hex) || Theme.ACCENTS.cyan;
  const hsv = Theme.hexToHsv(normalized);
  accentHex.value = normalized;
  accentH.value = hsv.h; accentS.value = hsv.s; accentV.value = hsv.v;
  $('accent-h-value').value = `${hsv.h}°`;
  $('accent-s-value').value = `${hsv.s}%`;
  $('accent-v-value').value = `${hsv.v}%`;
  $('custom-color-preview').style.background = normalized;
  document.querySelector('.custom-swatch').style.setProperty('--sw', normalized);
}

function updateCustomAccentFromHsv() {
  const hex = Theme.hsvToHex(accentH.value, accentS.value, accentV.value);
  prefs.customAccent = hex;
  syncCustomColorControls(hex);
  Theme.applyAccent(hex);
  savePrefs();
}

[accentH, accentS, accentV].forEach(slider => slider.addEventListener('input', updateCustomAccentFromHsv));
accentHex.addEventListener('input', () => {
  const hex = Theme.normalizeHex(accentHex.value);
  accentHex.classList.toggle('invalid', !hex);
  if (!hex) return;
  prefs.customAccent = hex;
  syncCustomColorControls(hex);
  Theme.applyAccent(hex);
  savePrefs();
});
initToggle('toggle-open-folder', v => { prefs.openFolder = v; savePrefs(); });
initToggle('toggle-remember',    v => { prefs.rememberSettings = v; savePrefs(); });
initToggle('toggle-start-on-add', v => { prefs.startOnAdd = v; savePrefs(); });
initToggle('toggle-minimize-tray', v => {
  prefs.minimizeToTray = v;
  invoke('set_minimize_to_tray', { enabled:v });
  savePrefs();
});

// ── Option groups ─────────────────────────────────────────────────────────────
function initGroup(group, onChange) {
  document.querySelectorAll(`[data-group="${group}"]`).forEach(btn => {
    btn.addEventListener('click', () => {
      document.querySelectorAll(`[data-group="${group}"]`).forEach(b => b.classList.remove('active'));
      btn.classList.add('active'); onChange(btn.dataset.value); savePrefs();
    });
  });
}

const CODEC_DEFAULTS = {
  x264:{ codec:'libx264' }, x265:{ codec:'libx265' },
};
const isAv1Codec = codec => codec === 'libsvtav1' || codec.startsWith('av1_');
function selectCodec(value, codec, qualityMode = 'crf', gameplayMode = 'native') {
  state.codec = codec; state.codec_name = value; state.quality_mode = qualityMode;
  state.gpu_gameplay_mode = gameplayMode;
  activateGroup('codec', value);
  if (!isAv1Codec(codec) && state.container === 'webm') {
    state.container = 'mp4'; activateGroup('container', 'mp4');
  }
  syncContainerUi(); savePrefs();
  syncPresetUi();
  syncAlgorithmUi();
}
initGroup('codec', v => {
  const d = CODEC_DEFAULTS[v];
  if (d) selectCodec(v, d.codec, 'crf');
});
initGroup('container', v => {
  if (v === 'webm' && !isAv1Codec(state.codec)) { activateGroup('container', state.container); return; }
  state.container = v;
});
function syncContainerUi() {
  const webm = document.querySelector('[data-group="container"][data-value="webm"]');
  if (!webm) return;
  const available = isAv1Codec(state.codec);
  if (!available && state.container === 'webm') {
    state.container = 'mp4'; activateGroup('container', 'mp4');
  }
  webm.disabled = !available;
  webm.title = available ? 'WebM with Opus audio' : 'Select a supported AV1 encoder first';
}
function syncPresetUi() {
  const isSvt = state.codec === 'libsvtav1';
  $('standard-preset-options').hidden = isSvt;
  $('svt-preset-options').hidden = !isSvt;
  if (isSvt) {
    $('svt-preset-slider').value = String(state.svt_preset);
    $('svt-preset-value').textContent = String(state.svt_preset);
    syncPresetSliderFill();
  }
}
function syncAlgorithmUi() {
  const dynamic = state.algorithm === 'dynamic';
  $('fixed-size-options').hidden = dynamic;
  $('dynamic-quality-options').hidden = !dynamic;
  $('allocation-controls').hidden = state.codec === 'libsvtav1';
  const nativeScenes = ['libx264','libx265'].includes(state.codec);
  const probeButton = document.querySelector('[data-group="allocation"][data-value="probe"]');
  probeButton.disabled = !nativeScenes;
  probeButton.title = nativeScenes
    ? 'Use a constant-quality probe to measure scene difficulty'
    : 'GPU APIs do not expose arbitrary custom scene zones';
  $('allocation-hint').textContent = !nativeScenes
    ? 'GPU encoders use their tested hardware lookahead and adaptive-quality controls; arbitrary custom scene zones are not exposed.'
    : state.allocation_mode === 'probe'
      ? 'Runs a fast constant-quality probe to estimate which gameplay scenes need more bits. Slower, but more precise.'
      : 'Analyzes motion across the video and gives difficult gameplay scenes more of the fixed bitrate budget.';
}
initGroup('algorithm', v => { state.algorithm=v; syncAlgorithmUi(); });
initGroup('allocation', v => {
  state.allocation_mode = v;
  syncAlgorithmUi();
});
const qualitySlider=$('quality-slider'), qualityInput=$('quality-input');
function setQuality(value) {
  const q=Math.max(0,Math.min(51,parseInt(value,10)||0));
  state.crf=q; qualitySlider.value=String(q); qualityInput.value=String(q); savePrefs();
}
qualitySlider.addEventListener('input', () => setQuality(qualitySlider.value));
qualityInput.addEventListener('change', () => setQuality(qualityInput.value));
initGroup('res', v => {
  const m = { 'auto':{res_mode:'auto',res_w:null,res_h:null}, '1440p':{res_mode:'scale',res_w:2560,res_h:1440}, '1080p':{res_mode:'scale',res_w:1920,res_h:1080},
               '720p':{res_mode:'scale',res_w:1280,res_h:720}, '540p':{res_mode:'scale',res_w:960,res_h:540},
               'original':{res_mode:'original',res_w:null,res_h:null} };
  Object.assign(state, m[v], { resolution_name:v });
});
const customFpsRow=$('custom-fps-row'), customFpsInput=$('custom-fps-input');
initGroup('fps', v => {
  if (v==='custom') { customFpsRow.classList.add('visible'); return; }
  customFpsRow.classList.remove('visible');
  if (v==='auto') { state.fps=0; state.fps_name='auto'; return; }
  const fps=parseFloat(v); state.fps=fps; state.fps_name=`${v}fps`;
});
customFpsInput.addEventListener('change', () => {
  const fps=parseFloat(customFpsInput.value)||60; state.fps=fps; state.fps_name=`${fps}fps`; savePrefs();
});
const customSizeRow=$('custom-size-row'), customSizeInput=$('custom-size-input');
initGroup('size', v => {
  if (v==='custom') { customSizeRow.classList.add('visible'); return; }
  customSizeRow.classList.remove('visible'); state.target_size=parseFloat(v); state.size_name=`${v}mb`;
});
customSizeInput.addEventListener('change', () => {
  const mb=parseFloat(customSizeInput.value)||8; state.target_size=mb; state.size_name=`${mb}mb`; savePrefs();
});
initGroup('preset', v => { state.preset=v; });
const svtPresetSlider = $('svt-preset-slider');
function syncPresetSliderFill() {
  const min = Number(svtPresetSlider.min) || 0;
  const max = Number(svtPresetSlider.max) || 13;
  const progress = ((Number(svtPresetSlider.value) - min) / (max - min)) * 100;
  svtPresetSlider.style.setProperty('--preset-progress', `${progress}%`);
}
svtPresetSlider.addEventListener('input', () => {
  state.svt_preset = parseInt(svtPresetSlider.value, 10);
  $('svt-preset-value').textContent = String(state.svt_preset);
  syncPresetSliderFill();
  savePrefs();
});

// ── File management ───────────────────────────────────────────────────────────
const VIDEO_EXTS = new Set(['mp4','mkv','avi','mov','webm','flv','m4v','wmv','ts','m2ts']);

function addFiles(paths) {
  // Remove any completed/errored items before adding fresh files
  const hadDone = state.files.some(f => f.status === 'done' || f.status === 'error');
  if (hadDone) {
    state.files = state.files.filter(f => f.status !== 'done' && f.status !== 'error');
  }

  let added = 0;
  for (const p of paths) {
    const name = p.split(/[\\/]/).pop();
    if (!VIDEO_EXTS.has(name.split('.').pop().toLowerCase())) continue;
    if (state.files.find(f => f.path === p)) continue;
    state.files.push({ path:p, name, status:'queued', detail:'', thumbUrl:null });
    added++;
  }

  if (added > 0 || hadDone) {
    renderQueue();
    // Kick off thumbnail loading for files that don't have one yet
    state.files.forEach((f, i) => { if (!f.thumbUrl) loadThumbnail(f, i); });
    if (added > 0 && prefs.startOnAdd && !state.running) {
      setTimeout(() => { if (!state.running && state.files.some(file => file.status === 'queued')) compressBtn.click(); }, 0);
    }
  }
}

// ── Thumbnail loading ─────────────────────────────────────────────────────────
async function loadThumbnail(fileObj, idx) {
  try {
    const dataUrl = await invoke('get_thumbnail', { filePath: fileObj.path });
    fileObj.thumbUrl = dataUrl;
    // Update DOM if the row still exists (queue may have re-rendered)
    const thumb = $(`thumb-${idx}`);
    if (thumb) {
      thumb.style.backgroundImage = `url('${dataUrl}')`;
      thumb.classList.add('loaded');
    }
  } catch { /* thumbnails are non-critical, fail silently */ }
}

// ── Queue rendering ───────────────────────────────────────────────────────────
function renderQueue() {
  queue.innerHTML = '';
  if (!state.files.length) {
    queue.innerHTML = '<div class="empty-state">No files queued — drop videos above or click to browse</div>';
    summary.textContent = ''; compressBtn.disabled = true; return;
  }
  compressBtn.disabled = false;

  state.files.forEach((f, i) => {
    const row = Object.assign(document.createElement('div'), {
      className: `queue-row ${f.status}`,
      id: `row-${i}`,
    });

    const thumbBg = f.thumbUrl ? `style="background-image:url('${f.thumbUrl}')"` : '';
    const statusText = f.status === 'done' ? '✓ Done' : f.status === 'error' ? 'Failed'
      : f.status === 'cancelled' ? 'Cancelled' : f.status === 'active' ? 'Compressing' : 'Queued';
    const isCancellable = state.running && (f.status === 'active' || f.status === 'queued');
    const rowAction = isCancellable ? 'Cancel' : 'Remove';
    const actionIcon = isCancellable
      ? '<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="7" y="7" width="10" height="10" rx="1"/></svg>'
      : '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18"/></svg>';

    row.innerHTML = `
      <div class="row-fill" id="fill-${i}"></div>
      <div class="row-thumb ${f.thumbUrl ? 'loaded' : ''}" id="thumb-${i}" ${thumbBg}></div>
      <div class="row-body">
        <div class="row-top">
          <span class="row-name" title="${f.path}">${f.name}</span>
          <span class="row-status" id="status-${i}">${statusText}</span>
        </div>
        <div class="row-detail" id="detail-${i}">${f.detail}</div>
      </div>
      <button class="queue-row-action ${isCancellable ? 'cancel-action' : 'remove-action'}" title="${rowAction}" aria-label="${rowAction} ${f.name}">
        ${actionIcon}
      </button>`;

    const handleRowAction = async () => {
      if (state.running && (f.status === 'active' || f.status === 'queued')) {
        f.status = 'cancelled'; f.detail = 'Cancellation requested';
        await invoke('cancel_file', { fileIndex:i }); renderQueue(); return;
      }
      row.classList.add('removing');
      setTimeout(() => { state.files.splice(i, 1); renderQueue(); }, 150);
    };
    row.querySelector('.queue-row-action').addEventListener('click', event => {
      event.stopPropagation();
      handleRowAction();
    });
    row.addEventListener('click', event => {
      if (document.documentElement.dataset.uiTheme !== 'studio' || event.target.closest('.queue-row-action')) return;
      handleRowAction();
    });

    queue.appendChild(row);
  });

  const n = state.files.length;
  summary.textContent = `${n} file${n !== 1 ? 's' : ''} queued`;
}

// ── Drag-drop events ──────────────────────────────────────────────────────────
listen('tauri://drag-enter', () => {
  dropZone.classList.add('drag-over');
  const hasDone = state.files.some(f => f.status === 'done' || f.status === 'error');
  if (hasDone) {
    dropSub.textContent = 'Completed items will be removed';
    dropSub.style.color  = 'var(--warning)';
    document.querySelectorAll('.queue-row.done, .queue-row.errored')
            .forEach(r => r.classList.add('clearing'));
  }
});

listen('tauri://drag-leave', () => {
  dropZone.classList.remove('drag-over');
  dropSub.textContent = DROP_SUB_DEFAULT;
  dropSub.style.color  = '';
  document.querySelectorAll('.queue-row.clearing').forEach(r => r.classList.remove('clearing'));
});

listen('tauri://drag-drop', e => {
  dropZone.classList.remove('drag-over');
  dropSub.textContent = DROP_SUB_DEFAULT;
  dropSub.style.color  = '';
  addFiles(e.payload?.paths ?? []);
});

dropZone.addEventListener('click', async () => {
  const files = await invoke('select_files');
  if (files?.length) addFiles(files);
});

// ── FFmpeg log ────────────────────────────────────────────────────────────────
listen('ffmpeg-log', e => {
  const line = e.payload?.line ?? '';
  const el   = Object.assign(document.createElement('div'), { className:'log-line', textContent:line });
  if      (line.startsWith('[DONE]'))  el.classList.add('done');
  else if (line.startsWith('[ERROR]')) el.classList.add('err');
  else if (line.startsWith('[INFO]') || line.startsWith('──')) el.classList.add('info');
  logOutput.appendChild(el);
  logOutput.scrollTop = logOutput.scrollHeight;
});
$('clear-log-btn').addEventListener('click', () => { logOutput.innerHTML = ''; });
$('copy-log-btn').addEventListener('click', async () => {
  const button = $('copy-log-btn');
  try {
    await navigator.clipboard.writeText(logOutput.innerText || '');
    button.textContent = 'Copied';
  } catch {
    button.textContent = 'Unavailable';
  }
  setTimeout(() => { button.textContent = 'Copy'; }, 1200);
});
$('collapse-log-btn').addEventListener('click', () => {
  const collapsed = $('log-section').classList.toggle('collapsed');
  $('collapse-log-btn').title = collapsed ? 'Expand FFmpeg log' : 'Collapse FFmpeg log';
});
$('clear-btn').addEventListener('click', () => { if (state.running) return; state.files = []; renderQueue(); });

function syncOutputFolderUi() {
  const path = prefs.outputFolder || 'Same as source';
  $('output-folder-path').textContent = path;
  $('output-folder-path').title = path;
}

$('output-folder-btn').addEventListener('click', async () => {
  const selected = await invoke('select_output_folder');
  if (selected) {
    prefs.outputFolder = selected;
    syncOutputFolderUi();
    savePrefs();
  }
});
$('persist-output-folder').addEventListener('change', event => {
  prefs.persistOutputFolder = event.currentTarget.checked;
  savePrefs();
});
$('open-output-folder-btn').addEventListener('click', () => {
  const path = prefs.outputFolder || state.files[0]?.path;
  if (path) invoke('open_folder', { path });
});

// ── Progress events ───────────────────────────────────────────────────────────
listen('progress', e => {
  const { file_index:i, percent, stage } = e.payload;
  const f = state.files[i]; if (!f) return;

  const row    = $(`row-${i}`);
  const fill   = $(`fill-${i}`);
  const status = $(`status-${i}`);
  const detail = $(`detail-${i}`);

  if (percent <= -2) {
    f.status = 'cancelled'; f.detail = 'Cancelled';
    if (row) row.className = 'queue-row cancelled';
    if (status) status.textContent = 'Cancelled';
    if (detail) detail.textContent = 'Cancelled';
  } else if (percent < 0) {
    f.status = 'error'; f.detail = stage;
    if (row)    row.className    = `queue-row error`;
    if (status) status.textContent = 'Failed';
    if (detail) detail.textContent = stage;
  } else if (percent >= 100) {
    f.status = 'done';
    if (row)    row.className    = 'queue-row done';
    if (fill)   fill.style.width = '100%';
    if (status) status.textContent = '✓ Done';
  } else {
    f.status = 'active';
    if (row)    row.className    = 'queue-row active';
    if (fill)   fill.style.width = `${percent}%`;
    if (status) status.textContent = `${percent.toFixed(0)}%`;
    if (detail && detail.textContent !== stage) { detail.textContent = stage; f.detail = stage; }
  }
});

// ── Compress ──────────────────────────────────────────────────────────────────
function setCompressLabel(label) {
  const text = compressBtn.querySelector('.compress-label');
  if (text) text.textContent = label;
}

$('compress-btn').addEventListener('click', async () => {
  if (state.running) {
    setCompressLabel('Cancelling…'); compressBtn.disabled = true;
    await invoke('cancel_all_compressions');
    return;
  }
  if (!state.files.length) return;
  state.running = true; compressBtn.disabled = false;
  setCompressLabel('Compressing…');
  compressBtn.classList.add('cancelling');
  compressBtn.title = 'Click to cancel all compressions';
  queue.classList.add('is-running');
  renderQueue();

  const options = {
    codec:state.codec, codec_name:state.codec_name, crf:state.crf,
    quality_mode:state.quality_mode, gpu_gameplay_mode:state.gpu_gameplay_mode,
    container:state.container, algorithm:state.algorithm,
    res_mode:state.res_mode, res_w:state.res_w, res_h:state.res_h,
    resolution_name:state.resolution_name,
    fps:state.fps, fps_name:state.fps_name,
    target_size:state.target_size, size_name:state.size_name,
    allocation_mode:state.codec === 'libsvtav1' ? 'lightweight' : state.allocation_mode,
    preset:state.preset, svt_preset:state.svt_preset,
    output_folder:prefs.outputFolder || null,
  };

  try {
    const results = await invoke('compress_files', { files:state.files.map(f=>f.path), options });
    const ok = results.filter(r=>r.success).length;
    const cancelled = results.filter(r=>r.cancelled).length;
    const err = results.filter(r=>!r.success && !r.cancelled).length;

    results.forEach(r => {
      const f = state.files[r.file_index];
      if (f && r.success) {
        const mb = (r.output_size/1024/1024).toFixed(2);
        f.detail = `${mb} MB  ·  ${r.output_path.split(/[\\/]/).pop()}`;
        const d = $(`detail-${r.file_index}`); if (d) d.textContent = f.detail;
      }
    });

    summary.textContent = `Done — ${ok} succeeded${err ? `, ${err} failed` : ''}${cancelled ? `, ${cancelled} cancelled` : ''}`;
    if (prefs.openFolder) {
      const first = results.find(r=>r.success);
      if (first) invoke('open_folder', { path:first.output_path });
    }
    invoke('play_sound');
  } catch (err) {
    summary.textContent = `Error: ${err}`;
  }

  setCompressLabel('Compress');
  compressBtn.disabled    = false;
  compressBtn.classList.remove('cancelling');
  compressBtn.title = '';
  queue.classList.remove('is-running');
  state.running = false;
  renderQueue();
});

function appendHardwareEncoders(encoders) {
  const group = $('codec-options');
  encoders.forEach(enc => {
    const btn = document.createElement('button');
    btn.className = 'option-btn hardware-codec';
    btn.dataset.group = 'codec'; btn.dataset.value = enc.codec_name;
    btn.innerHTML = `<span>${enc.label}</span><span class="badge gpu-badge" title="${enc.gpu}">${enc.gpu}</span>`;
    btn.addEventListener('click', () => selectCodec(enc.codec_name, enc.codec, enc.quality_mode, enc.gameplay_mode));
    group.appendChild(btn);
  });
  const supported = encoders.find(e => e.codec_name === state.codec_name);
  if (supported) selectCodec(supported.codec_name, supported.codec, supported.quality_mode, supported.gameplay_mode);
  else if (!['x264','x265'].includes(state.codec_name)) selectCodec('x264','libx264');
  const status = $('encoder-detect-status');
  status.textContent = encoders.length ? `${encoders.length} additional encoder${encoders.length===1?'':'s'} available` : 'No additional encoders detected';
}

async function refreshHardwareEncoders() {
  $('encoder-detect-status').textContent = 'Testing additional encoders…';
  try { appendHardwareEncoders(await invoke('detect_hardware_encoders')); }
  catch { $('encoder-detect-status').textContent = 'Additional encoder detection unavailable'; }
}

function renderFfmpegHealth(r) {
  const ok = !!(r.ffmpeg_ok && r.ffprobe_ok);
  const essentials = r.build_variant === 'essentials';
  const needsUpdate = !!r.update_available;
  const missingSvt = ok && !r.svt_version;
  const hasWarning = !ok || essentials || missingSvt || needsUpdate;
  ffmpegAlert.hidden = !hasWarning;
  $('ffmpeg-menu-path').textContent = r.path || 'No executable detected';
  $('update-ffmpeg-btn').textContent = ok && !hasWarning ? 'Reinstall Full build' : (ok ? 'Update FFmpeg' : 'Install FFmpeg Full');
  if (!ok) {
    $('ffmpeg-menu-title').textContent = 'FFmpeg is missing';
    $('ffmpeg-menu-detail').textContent = 'Install the current Full build to enable every supported encoder.';
  } else if (essentials) {
    $('ffmpeg-menu-title').textContent = 'Essentials build detected';
    $('ffmpeg-menu-detail').textContent = 'SVT-AV1 is not included. Update to the Full build for all Effigy encoders.';
  } else if (missingSvt) {
    $('ffmpeg-menu-title').textContent = 'SVT-AV1 is unavailable';
    $('ffmpeg-menu-detail').textContent = 'Install the Full build to enable the SVT-AV1 encoder.';
  } else if (needsUpdate) {
    $('ffmpeg-menu-title').textContent = 'FFmpeg update available';
    const svtNote = r.svt_version ? ` Embedded SVT-AV1: v${r.svt_version}.` : '';
    const latestNote = r.latest_version ? ` Latest FFmpeg: ${r.latest_version}.` : '';
    $('ffmpeg-menu-detail').textContent = `This build is outdated.${svtNote}${latestNote}`;
  } else {
    $('ffmpeg-menu-title').textContent = r.full_build ? 'FFmpeg Full is current' : 'Custom FFmpeg build';
    $('ffmpeg-menu-detail').textContent = r.full_build ? 'The active Full build is up to date.' : 'This is not identified as a Gyan Full or Essentials build.';
  }
}

ffmpegStatusBtn.addEventListener('click', event => {
  event.stopPropagation();
  ffmpegMenu.hidden = !ffmpegMenu.hidden;
  ffmpegStatusBtn.setAttribute('aria-expanded', String(!ffmpegMenu.hidden));
});
ffmpegMenu.addEventListener('click', event => event.stopPropagation());
document.addEventListener('click', () => {
  ffmpegMenu.hidden = true;
  ffmpegStatusBtn.setAttribute('aria-expanded', 'false');
});
$('update-ffmpeg-btn').addEventListener('click', () => {
  ffmpegMenu.hidden = true;
  ffmpegStatusBtn.setAttribute('aria-expanded', 'false');
  $('ffmpeg-warning-title').textContent = 'Update FFmpeg';
  $('ffmpeg-warning-body').textContent = 'Effigy will install the checksum-verified current Full build in its managed FFmpeg folder. Unrelated FFmpeg installations are not overwritten.';
  $('install-ffmpeg-btn').textContent = 'Update FFmpeg';
  $('install-idle').style.display = ffmpegInstallRunning ? 'none' : 'block';
  $('install-progress').style.display = ffmpegInstallRunning ? 'block' : 'none';
  ffmpegWarning.classList.add('visible');
});
// ── FFmpeg check & install ────────────────────────────────────────────────────
(async () => {
  const r = await invoke('check_ffmpeg');
  const ok = r.ffmpeg_ok && r.ffprobe_ok;
  renderFfmpegHealth(r);
  ffmpegDot.className = 'status-dot ' + (ok ? 'ok' : 'err');
  if (ok) {
    const ver = r.ffmpeg_version.replace('ffmpeg version ','').split(' ')[0];
    ffmpegLabel.textContent = `ffmpeg ${ver}${r.source==='local' ? ' (local)' : ''}`;
    await refreshHardwareEncoders();
  } else {
    ffmpegLabel.textContent = 'ffmpeg missing';
    ffmpegWarning.classList.add('visible');
  }
})();

$('skip-install-btn').addEventListener('click', () => ffmpegWarning.classList.remove('visible'));
$('close-install-btn').addEventListener('click', () => ffmpegWarning.classList.remove('visible'));

let ffmpegInstallRunning = false;
function showInstallFailure(message) {
  const statusEl = $('install-status');
  statusEl.textContent = `Update failed: ${message}`;
  statusEl.style.color = 'var(--error)';
  $('retry-install-btn').hidden = false;
  $('install-bar-fill').classList.add('failed');
}

async function runFfmpegInstall() {
  if (ffmpegInstallRunning) return;
  ffmpegInstallRunning = true;
  $('install-idle').style.display = 'none';
  $('install-progress').style.display = 'block';
  $('retry-install-btn').hidden = true;
  const statusEl = $('install-status');
  const barFill = $('install-bar-fill');
  statusEl.textContent = 'Preparing...';
  statusEl.style.color = '';
  barFill.className = 'install-bar-fill';
  let completed = false;

  const unlisten = await listen('ffmpeg-install', async event => {
    const { step, done, error } = event.payload;
    if (error) {
      showInstallFailure(error);
    } else if (done) {
      completed = true;
      statusEl.textContent = 'Installed. Verifying...';
      barFill.classList.add('done');
      const refreshed = await invoke('check_ffmpeg');
      renderFfmpegHealth(refreshed);
      if (refreshed.ffmpeg_ok && refreshed.ffprobe_ok) {
        ffmpegDot.className = 'status-dot ok';
        const ver = refreshed.ffmpeg_version.replace('ffmpeg version ','').split(' ')[0];
        ffmpegLabel.textContent = `ffmpeg ${ver} (local)`;
        ffmpegWarning.classList.remove('visible');
        await refreshHardwareEncoders();
      }
    } else {
      statusEl.textContent = step;
    }
  });

  try {
    await invoke('install_ffmpeg');
  } catch (error) {
    showInstallFailure(error);
  } finally {
    ffmpegInstallRunning = false;
    unlisten();
    if (!completed && !$('install-status').textContent.startsWith('Update failed:')) {
      showInstallFailure('The installer stopped before completing.');
    }
  }
}

$('install-ffmpeg-btn').addEventListener('click', runFfmpegInstall);
$('retry-install-btn').addEventListener('click', runFfmpegInstall);
// ── Init ──────────────────────────────────────────────────────────────────────
(async function initializeApp() {
  try {
    await loadPrefs();
    applyLoadedPrefs();
    renderQueue();
    savePrefs();
  } finally {
    // Hidden WebViews may throttle animation frames, so do not await rAF here.
    await new Promise(resolve => setTimeout(resolve, 50));
    await invoke('show_ready_window');
  }
})();

// ── Custom window controls ────────────────────────────────────────────────────
(async () => {
  const tWin = window.__TAURI__?.window;
  if (!tWin) return;
  const appWin = (tWin.getCurrentWindow ?? tWin.getCurrent)?.();
  if (!appWin) return;

  const ICON_MAX = `<svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.1"><rect x=".6" y=".6" width="8.8" height="8.8"/></svg>`;
  const ICON_RST = `<svg width="11" height="11" viewBox="0 0 11 11" fill="none" stroke="currentColor" stroke-width="1.1"><rect x="3" y=".5" width="7.5" height="7.5"/><path d=".5,3.5 v7 h7.5" stroke-linecap="round"/></svg>`;

  async function syncMaxIcon() {
    const isMax = await appWin.isMaximized();
    const btn = $('wc-maximize');
    btn.innerHTML = isMax ? ICON_RST : ICON_MAX;
    btn.title     = isMax ? 'Restore' : 'Maximize';
  }

  $('wc-minimize').addEventListener('click', () => appWin.minimize());
  $('wc-maximize').addEventListener('click', async () => { await appWin.toggleMaximize(); syncMaxIcon(); });
  const showCloseWarning = () => $('close-warning').classList.add('visible');
  $('wc-close').addEventListener('click', async () => {
    try {
      await invoke('request_titlebar_close');
    } catch (error) {
      addLog(`Could not close the application: ${error}`, 'err');
    }
  });
  $('keep-running-btn').addEventListener('click', () => $('close-warning').classList.remove('visible'));
  $('confirm-close-btn').addEventListener('click', async () => {
    $('confirm-close-btn').disabled = true;
    $('confirm-close-btn').textContent = 'Cancelling…';
    await invoke('cancel_all_and_close');
  });
  await listen('close-requested-during-compression', showCloseWarning);

  syncMaxIcon();
  window.addEventListener('resize', syncMaxIcon);
})();
