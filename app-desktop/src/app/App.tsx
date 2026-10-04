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
  onMount(() => {
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
