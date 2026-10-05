import { invoke, isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { registerSaveTask, flushPendingSaves } from './save-guard';
import { flushAllVisualSettings } from '../features/overlays/overlay-state';
import { setNotice } from './project-state';
import { Route, Router } from '@solidjs/router';
import { createEffect, createSignal, lazy, onCleanup, onMount } from 'solid-js';
import type { JSX } from 'solid-js';
import { listen } from '@tauri-apps/api/event';
import {
  appStartupReady, setAppStartupReady, setTranscriptionEvent, setTranslationEvent, setTtsEvent,
} from './project-state';
import type { TranscriptionEvent, TranslationJobEvent, TtsJobEvent } from '../features/transcript/types';
import StartPage from '../features/projects/StartPage';
const WorkspaceLayout = lazy(() => import('../features/workspace/WorkspaceLayout'));
const SettingsView = lazy(() => import('../features/workspace/settings/SettingsView'));
import StartupSplash from './StartupSplash';
import Toast from '../shared/components/Toast';
import JobsDock from '../shared/components/JobsDock';

function ApplicationFrame(props: { children?: JSX.Element }) {
  return <><div class="app-frame">{props.children}</div><JobsDock /><Toast /></>;
}

function NotFound() {
  onMount(() => setAppStartupReady(true));
  return <main class="not-found"><div class="brand-mark"><span>404</span></div><h1>Không tìm thấy trang này</h1><p>Đường dẫn có thể đã thay đổi hoặc dự án không tồn tại.</p><a class="button button-primary" href="/">Quay lại Project Manager</a></main>;
}

export default function App() {
  const [minimumSplashElapsed, setMinimumSplashElapsed] = createSignal(false);
  const [splashLeaving, setSplashLeaving] = createSignal(false);
  const [splashVisible, setSplashVisible] = createSignal(true);
  let unlistenTranscription: (() => void) | undefined;
  let unlistenTranslation: (() => void) | undefined;
  let unlistenTts: (() => void) | undefined;
  let minimumSplashTimer: number | undefined;
  let dismissSplashTimer: number | undefined;
  let alive = true;
  let allowClose = false;
  let closing = false;
  let unlistenJobWarning: (()=>void) | undefined;
  let unlistenClose: (() => void) | undefined;
  const unregisterVisualSave = registerSaveTask('visual-settings', flushAllVisualSettings);
  const reportError = (kind: string, message: string) => { if (isTauri()) void invoke('report_frontend_error', { kind, message }).catch(() => undefined); };
  const onError = (event: ErrorEvent) => reportError('error', event.message);
  const onRejection = (event: PromiseRejectionEvent) => reportError('unhandledrejection', event.reason instanceof Error ? event.reason.message : 'Unhandled rejection');
  const onRecovery = (event: Event) => setNotice({kind:'info',message:(event as CustomEvent<string>).detail});
  onMount(() => {
    window.addEventListener('error', onError);
    window.addEventListener('unhandledrejection', onRejection);
    window.addEventListener('jc-data-recovered', onRecovery);
    if (isTauri()) {
      void getCurrentWindow().onCloseRequested(event => {
        if (allowClose) return;
        event.preventDefault();
        if (closing) return;
        closing = true;
        void flushPendingSaves().then(() => { allowClose = true; return getCurrentWindow().close(); })
          .catch((error: unknown) => { closing = false; allowClose = false; setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Chưa lưu được thay đổi. Ứng dụng được giữ mở để thử lại.' }); });
      }).then(unlisten => { if (alive) unlistenClose = unlisten; else unlisten(); }).catch(() => setNotice({kind:'error', message:'Không thiết lập được kiểm tra lưu khi đóng. Hãy lưu thay đổi trước khi thoát ứng dụng.'}));
      void invoke<{previousSessionInterrupted: boolean}>('get_session_status').then(status => {
        if (status.previousSessionInterrupted) setNotice({ kind: 'info', message: 'Phiên trước đã dừng bất ngờ. Các tác vụ chưa xong được đánh dấu gián đoạn; bạn có thể thử lại trong hàng đợi.' });
      }).catch(() => undefined);
    }
    void listen<string>('job-history-warning',event => setNotice({kind:'error',message:event.payload})).then(unlisten => {if(alive) unlistenJobWarning=unlisten; else unlisten();}).catch(() => undefined);
    minimumSplashTimer = window.setTimeout(() => setMinimumSplashElapsed(true), 520);
    void listen<TranscriptionEvent>('transcription-progress', (event) => setTranscriptionEvent(event.payload))
      .then((unlisten) => {
        if (alive) unlistenTranscription = unlisten;
        else unlisten();
      })
      .catch(() => undefined);
    void listen<TranslationJobEvent>('translation-progress', (event) => setTranslationEvent(event.payload))
      .then((unlisten) => {
        if (alive) unlistenTranslation = unlisten;
        else unlisten();
      })
      .catch(() => undefined);
    void listen<TtsJobEvent>('tts-progress', (event) => setTtsEvent(event.payload))
      .then((unlisten) => {
        if (alive) unlistenTts = unlisten;
        else unlisten();
      })
      .catch(() => undefined);
  });
  createEffect(() => {
    if (!appStartupReady() || !minimumSplashElapsed() || splashLeaving()) return;
    setSplashLeaving(true);
    dismissSplashTimer = window.setTimeout(() => setSplashVisible(false), 180);
  });
  onCleanup(() => {
    alive = false;
    unregisterVisualSave();
    unlistenClose?.();
    unlistenJobWarning?.();
    window.removeEventListener('error', onError);
    window.removeEventListener('unhandledrejection', onRejection);
    window.removeEventListener('jc-data-recovered', onRecovery);
    if (minimumSplashTimer !== undefined) window.clearTimeout(minimumSplashTimer);
    if (dismissSplashTimer !== undefined) window.clearTimeout(dismissSplashTimer);
    unlistenTranscription?.();
    unlistenTranslation?.();
    unlistenTts?.();
  });
  return <>
    <div class="application-host" inert={splashVisible()} aria-hidden={splashVisible() ? 'true' : undefined}>
      <Router root={ApplicationFrame}>
        <Route path="/" component={StartPage} />
        <Route path="/settings" component={SettingsView} />
        <Route path="/project/:projectId/:section" component={WorkspaceLayout} />
        <Route path="*404" component={NotFound} />
      </Router>
    </div>
    {splashVisible() && <StartupSplash leaving={splashLeaving()} />}
  </>;
}
