<script>
  import { onMount } from 'svelte';
  import appIcon from '../src-tauri/icons/icon.png';
  import './themes.js';

  onMount(() => {
    void import('./app-controller.js');
  });
</script>

<div id="app">

  <!-- ── Window controls ───────────────────────────────────── -->
  <div class="window-control-bar" data-tauri-drag-region>
    <div class="studio-title-brand" data-tauri-drag-region>
      <img id="studio-app-icon" src={appIcon} alt="" data-tauri-drag-region />
      <strong data-tauri-drag-region>Effigy</strong>
      <span data-tauri-drag-region>Video Compressor</span>
    </div>
    <span class="window-title" data-tauri-drag-region>Effigy Video Compressor</span>
    <div class="window-controls">
      <button class="wc-btn" id="wc-minimize" title="Minimize">
        <svg width="10" height="1" viewBox="0 0 10 1"><rect width="10" height="1.5" fill="currentColor"/></svg>
      </button>
      <button class="wc-btn" id="wc-maximize" title="Maximize">
        <svg id="wc-max-icon" width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.1"><rect x="0.6" y="0.6" width="8.8" height="8.8"/></svg>
      </button>
      <button class="wc-btn" id="wc-close" title="Close">
        <svg width="10" height="10" viewBox="0 0 10 10" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"><line x1="1" y1="1" x2="9" y2="9"/><line x1="9" y1="1" x2="1" y2="9"/></svg>
      </button>
    </div>
  </div>

  <!-- ── Header ───────────────────────────────────────────── -->
  <header data-tauri-drag-region>
    <div class="brand" data-tauri-drag-region>
      <img class="brand-app-icon" id="modern-app-icon" src={appIcon} alt="" data-tauri-drag-region />
      <span class="brand-name" data-tauri-drag-region>Effigy</span>
      <span class="brand-sep" data-tauri-drag-region>·</span>
      <span class="brand-sub" data-tauri-drag-region>Video Compressor</span>
    </div>
    <div class="titlebar-drag-region" data-tauri-drag-region></div>
    <div class="header-end">
      <div class="ffmpeg-status">
        <span class="status-dot" id="ffmpeg-dot"></span>
        <span class="status-label" id="ffmpeg-label">checking…</span>
      </div>
      <button class="icon-btn" id="settings-btn" title="Settings">
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none"
             stroke="currentColor" stroke-width="2.2" stroke-linecap="round">
          <line x1="3" y1="6"  x2="21" y2="6"/>
          <line x1="3" y1="12" x2="21" y2="12"/>
          <line x1="3" y1="18" x2="21" y2="18"/>
          <circle cx="8"  cy="6"  r="2.5" fill="currentColor" stroke="none"/>
          <circle cx="16" cy="12" r="2.5" fill="currentColor" stroke="none"/>
          <circle cx="8"  cy="18" r="2.5" fill="currentColor" stroke="none"/>
        </svg>
      </button>
    </div>
  </header>

  <!-- ── Main ─────────────────────────────────────────────── -->
  <main>

    <!-- Compression sidebar -->
    <aside id="sidebar">

      <div class="studio-sidebar-heading">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 7h10M4 17h10M17 5v4M7 15v4M4 12h16M20 10v4"/></svg>
        <span>Encode Options</span>
      </div>

      <div class="sidebar-section">
        <div class="section-label">Codec</div>
        <div class="option-group" id="codec-options">
          <button class="option-btn active" data-group="codec" data-value="x264">
            x264 <span class="badge">default</span>
          </button>
          <button class="option-btn" data-group="codec" data-value="x265">
            x265 <span class="badge">smaller</span>
          </button>
        </div>
        <p class="encoder-detect-status" id="encoder-detect-status">Detecting additional encoders…</p>
      </div>

      <div class="sidebar-section">
        <div class="section-label">File Container</div>
        <div class="option-group">
          <button class="option-btn active" data-group="container" data-value="mp4">
            MP4 <span class="badge">AAC audio</span>
          </button>
          <button class="option-btn" data-group="container" data-value="webm">
            WebM <span class="badge">Opus audio</span>
          </button>
        </div>
      </div>

      <div class="sidebar-section">
        <div class="section-label">Encoder Preset</div>
        <div id="standard-preset-options">
          <div class="option-group">
            <button class="option-btn" data-group="preset" data-value="medium">Medium</button>
            <button class="option-btn" data-group="preset" data-value="slow">Slow</button>
            <button class="option-btn active" data-group="preset" data-value="slower">Slower</button>
            <button class="option-btn" data-group="preset" data-value="veryslow">Slowest</button>
          </div>
        </div>
        <div id="svt-preset-options" hidden>
          <div class="preset-slider-row">
            <input id="svt-preset-slider" type="range" min="0" max="13" value="8" />
            <span id="svt-preset-value">8</span>
          </div>
          <div class="preset-scale"><span>Best quality</span><span>Fastest</span></div>
          <p class="quality-hint">Lower presets compress more slowly and usually preserve more quality.</p>
        </div>
      </div>

      <div class="sidebar-section">
        <div class="section-label">Resolution</div>
        <div class="option-group">
          <button class="option-btn active" data-group="res" data-value="auto">
            Auto <span class="badge">bitrate based</span>
          </button>
          <button class="option-btn" data-group="res" data-value="1440p">
            1440p <span class="badge">2560×1440</span>
          </button>
          <button class="option-btn" data-group="res" data-value="1080p">
            1080p <span class="badge">1920×1080</span>
          </button>
          <button class="option-btn" data-group="res" data-value="720p">
            720p <span class="badge">1280×720</span>
          </button>
          <button class="option-btn" data-group="res" data-value="540p">
            540p <span class="badge">960×540</span>
          </button>
          <button class="option-btn" data-group="res" data-value="original">
            Original <span class="badge">source</span>
          </button>
        </div>
      </div>

      <div class="sidebar-section">
        <div class="section-label">Frame Rate</div>
        <div class="option-group">
          <button class="option-btn active" data-group="fps" data-value="auto">
            Auto <span class="badge">bitrate based</span>
          </button>
          <button class="option-btn" data-group="fps" data-value="60">
            60 fps
          </button>
          <button class="option-btn" data-group="fps" data-value="30">30 fps</button>
          <button class="option-btn" data-group="fps" data-value="custom">Custom</button>
        </div>
        <div class="custom-input-row" id="custom-fps-row">
          <input id="custom-fps-input" type="number" min="1" max="240" placeholder="e.g. 24" />
        </div>
      </div>

      <div class="sidebar-section">
        <div class="section-label">File Size Algorithm</div>
        <div class="option-group algorithm-options">
          <button class="option-btn" data-group="algorithm" data-value="fixed">
            Fixed
          </button>
          <button class="option-btn active" data-group="algorithm" data-value="dynamic">
            CRF
          </button>
        </div>
        <div id="fixed-size-options" hidden>
          <div class="subsection-label">Target Size</div>
          <div class="option-group">
          <button class="option-btn active" data-group="size" data-value="8">
            8 MB <span class="badge">default</span>
          </button>
          <button class="option-btn" data-group="size" data-value="10">10 MB</button>
          <button class="option-btn" data-group="size" data-value="20">20 MB</button>
          <button class="option-btn" data-group="size" data-value="25">25 MB</button>
          <button class="option-btn" data-group="size" data-value="custom">Custom</button>
          </div>
          <div class="custom-input-row" id="custom-size-row">
            <div class="unit-wrapper">
              <input id="custom-size-input" type="number" min="1" max="9999" placeholder="e.g. 50" />
              <span class="unit-label">MB</span>
            </div>
          </div>
          <div class="subsection-label">Gameplay Allocation</div>
          <div class="option-group" id="allocation-options">
            <button class="option-btn active" data-group="allocation" data-value="lightweight">
              Lightweight <span class="badge">default</span>
            </button>
            <button class="option-btn" data-group="allocation" data-value="probe">
              Probe-based
            </button>
          </div>
          <p class="quality-hint" id="allocation-hint">
            Analyzes motion across the video and gives difficult gameplay scenes more of the fixed bitrate budget.
          </p>
        </div>
        <div id="dynamic-quality-options">
          <div class="subsection-label">CRF Quality</div>
          <div class="quality-control">
            <input id="quality-slider" type="range" min="0" max="51" value="24" />
            <input id="quality-input" type="number" min="0" max="51" value="24" />
          </div>
          <p class="quality-hint">Lower values preserve more detail and create larger files.</p>
        </div>
      </div>

    </aside>

    <!-- File panel -->
    <div id="file-panel">

      <div id="drop-zone">
        <svg width="28" height="28" viewBox="0 0 24 24" fill="none"
             stroke="currentColor" stroke-width="1.5" stroke-linecap="round"
             stroke-linejoin="round" class="drop-icon">
          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>
          <polyline points="17 8 12 3 7 8"/>
          <line x1="12" y1="3" x2="12" y2="15"/>
        </svg>
        <p class="drop-text">Drop videos here or click to browse</p>
        <p class="drop-sub">MP4 · MKV · MOV · AVI · WEBM · and more</p>
      </div>

      <div id="queue-header">
        <span class="queue-title">
          Queue <span id="queue-hint">— hover an item for actions</span>
        </span>
        <button class="ghost-btn danger" id="clear-btn">Clear all</button>
      </div>

      <div id="queue">
        <div class="empty-state">No files queued — drop videos above or click Browse</div>
      </div>

      <div id="log-section">
        <div id="log-header">
          <span class="section-label" style="margin:0">FFmpeg Log</span>
          <button class="ghost-btn" id="clear-log-btn">Clear</button>
          <button class="ghost-btn studio-only" id="copy-log-btn">Copy</button>
          <button class="icon-btn studio-only" id="collapse-log-btn" title="Collapse FFmpeg log" aria-label="Collapse FFmpeg log">
            <svg viewBox="0 0 24 24"><path d="m6 15 6-6 6 6"/></svg>
          </button>
        </div>
        <div id="log-output"></div>
      </div>

    </div>
  </main>

  <!-- ── Footer ───────────────────────────────────────────── -->
  <footer>
    <div class="studio-output-folder studio-only">
      <span class="output-label">Output Folder</span>
      <button id="output-folder-btn" title="Choose output folder">
        <span id="output-folder-path">Same as source</span>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 7h7l2 2h9v10H3V7Z"/></svg>
      </button>
      <button class="ghost-btn" id="open-output-folder-btn">Open Folder</button>
    </div>
    <span id="summary"></span>
    <button id="compress-btn" disabled>
      <svg class="compress-icon studio-only" viewBox="0 0 24 24" aria-hidden="true"><path d="M3 12h18M8 7l-5 5 5 5M16 7l5 5-5 5"/></svg>
      <span class="compress-label">Compress</span>
    </button>
  </footer>

</div><!-- /app -->

<!-- ── Settings panel ───────────────────────────────────── -->
<div id="settings-panel">
  <div class="sp-header">
    <span class="sp-title">Settings</span>
    <button class="icon-btn" id="close-settings-btn">✕</button>
  </div>
  <div class="sp-body">

    <div class="sp-section theme-section">
      <div class="section-label">Theme</div>
      <div class="theme-grid" id="theme-selector">
        <button class="theme-card active" data-ui-theme="studio">
          <span class="theme-preview preview-studio"><i></i><i></i><i></i></span>
          <span class="theme-copy"><strong>Modern</strong><small>Focused workspace</small></span>
        </button>
        <button class="theme-card" data-ui-theme="modern">
          <span class="theme-preview preview-modern"><i></i><i></i><i></i></span>
          <span class="theme-copy"><strong>Simple</strong><small>Soft and spacious</small></span>
        </button>
      </div>
    </div>

    <div class="sp-section">
      <div class="section-label">Accent Color</div>
      <div class="color-swatches">
        <button class="swatch"        data-theme="amber"  title="Amber"  style="--sw:#f59e0b"></button>
        <button class="swatch"        data-theme="blue"   title="Blue"   style="--sw:#3b82f6"></button>
        <button class="swatch"        data-theme="green"  title="Green"  style="--sw:#34d399"></button>
        <button class="swatch"        data-theme="purple" title="Purple" style="--sw:#a78bfa"></button>
        <button class="swatch active" data-theme="cyan"   title="Cyan"   style="--sw:#22d3ee"></button>
        <button class="swatch"        data-theme="rose"   title="Rose"   style="--sw:#fb7185"></button>
        <button class="swatch custom-swatch" data-theme="custom" title="Edit custom color" style="--sw:#22d3ee" aria-label="Edit custom accent color">
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 16.5V20h3.5L18.2 9.3l-3.5-3.5L4 16.5Zm16.7-9.7a.95.95 0 0 0 0-1.4l-2.1-2.1a.95.95 0 0 0-1.4 0l-1.7 1.7L19 8.5l1.7-1.7Z"/></svg>
        </button>
      </div>
      <div class="custom-color-panel" id="custom-color-panel" hidden>
        <div class="custom-color-top">
          <span class="custom-color-preview" id="custom-color-preview"></span>
          <label class="hex-field"><span>HEX</span><input id="accent-hex" type="text" maxlength="7" value="#22D3EE" spellcheck="false" /></label>
        </div>
        <label class="hsv-row"><span>Hue</span><input id="accent-h" type="range" min="0" max="359" value="188" /><output id="accent-h-value">188°</output></label>
        <label class="hsv-row"><span>Saturation</span><input id="accent-s" type="range" min="0" max="100" value="86" /><output id="accent-s-value">86%</output></label>
        <label class="hsv-row"><span>Value</span><input id="accent-v" type="range" min="0" max="100" value="93" /><output id="accent-v-value">93%</output></label>
      </div>
    </div>

    <div class="sp-section">
      <div class="section-label">Appearance</div>
      <div class="mode-selector">
        <button class="mode-btn" data-mode="system">System</button>
        <button class="mode-btn active" data-mode="dark">Dark</button>
        <button class="mode-btn" data-mode="light">Light</button>
      </div>
    </div>

    <div class="sp-divider"></div>

    <div class="sp-section">
      <div class="section-label">Behavior</div>
      <div class="toggle-row" id="row-start-on-add">
        <span class="toggle-label">Start compression on add</span>
        <div class="toggle-track" id="toggle-start-on-add" data-active="false">
          <div class="toggle-thumb"></div>
        </div>
      </div>
      <div class="toggle-row" id="row-minimize-tray">
        <span class="toggle-label">Minimize to tray when closing</span>
        <div class="toggle-track" id="toggle-minimize-tray" data-active="false">
          <div class="toggle-thumb"></div>
        </div>
      </div>
      <div class="toggle-row" id="row-open-folder">
        <span class="toggle-label">Open folder when done</span>
        <div class="toggle-track" id="toggle-open-folder" data-active="false">
          <div class="toggle-thumb"></div>
        </div>
      </div>
      <div class="toggle-row" id="row-remember">
        <span class="toggle-label">Remember encoder settings</span>
        <div class="toggle-track" id="toggle-remember" data-active="true">
          <div class="toggle-thumb"></div>
        </div>
      </div>
    </div>

    <div class="sp-credit">Made by <strong>JosEffigy</strong></div>

  </div>
</div>
<div id="settings-backdrop"></div>

<!-- ── FFmpeg warning overlay ────────────────────────────── -->
<div id="ffmpeg-warning">
  <div class="warn-card">
    <div class="warn-icon">⚠</div>
    <h2 class="warn-title">FFmpeg not found</h2>
    <p class="warn-body">
      FFmpeg is required but wasn't detected in your system PATH or app folder.
    </p>
    <div id="install-idle">
      <p class="warn-note">
        Would you like Effigy to download and install it automatically?<br>
        <span class="warn-size">FFmpeg Full · from gyan.dev</span>
      </p>
      <div class="warn-actions">
        <button class="btn-primary" id="install-ffmpeg-btn">⬇ Install FFmpeg</button>
        <button class="btn-ghost"   id="skip-install-btn">Skip for now</button>
      </div>
    </div>
    <div id="install-progress" style="display:none">
      <div class="install-step" id="install-status">Preparing…</div>
      <div class="install-bar"><div class="install-bar-fill" id="install-bar-fill"></div></div>
    </div>
  </div>
</div>

<!-- ── Active compression close warning ───────────────────── -->
<div id="close-warning">
  <div class="warn-card">
    <div class="warn-icon">⚠</div>
    <h2 class="warn-title">Compression is still running</h2>
    <p class="warn-body">Closing Effigy will cancel every active and queued compression. Partial output files may be removed.</p>
    <div class="warn-actions">
      <button class="btn-primary danger-action" id="confirm-close-btn">Cancel all and close</button>
      <button class="btn-ghost" id="keep-running-btn">Keep running</button>
    </div>
  </div>
</div>
