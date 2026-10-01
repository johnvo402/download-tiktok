import IntelWhisperCard from "./IntelWhisperCard";
import { createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { useNavigate, useSearchParams } from '@solidjs/router';
import { convertFileSrc } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import {
  AudioLines, Check, ChevronRight, CircleHelp, Cpu, FolderOpen, Globe2, Languages, LockKeyhole,
  ArrowLeft, Info, LoaderCircle, Monitor, SlidersHorizontal, Sparkles, Trash2, Volume2,
} from 'lucide-solid';
import { translationService, type AppSettings } from '../../../app/services/translation-service';
import { projectService } from '../../../app/services/project-service';
import { dialogueService, type DialogueEnvironment, type DialogueModelProgress } from '../../../app/services/dialogue-service';
import { ttsService } from '../../../app/services/tts-service';
import { formatBytes, storageService, type StorageUsage } from '../../../app/services/storage-service';
import { runtimeService, type RuntimeHealth } from '../../../app/services/runtime-service';
import type { TranslationProviderId } from '../../transcript/types';
import type { AsrEnvironment, OcrEnvironment, TextRemovalEnvironment, TtsEnvironment } from '../../transcript/types';
import { estimatedModelSize } from '../../transcript/transcript-utils';
import Modal from '../../../shared/components/Modal';
import { setAppStartupReady } from '../../../app/project-state';
import OptionalAiRuntimeCard from './OptionalAiRuntimeCard';
import IntelOcrRuntimeCard from './IntelOcrRuntimeCard';
import { optionalRuntimeService } from '../../../app/services/optional-runtime-service';

type SettingsSection = 'General' | 'Translation' | 'Speech Recognition' | 'Voice / TTS' | 'Advanced' | 'About';
const OCR_MODELS: Array<{ id: AppSettings['asr']['ocrModel']; label: string; hint: string; estimatedMb: number }> = [
  { id: 'tiny', label: 'PP-OCRv6 Tiny', hint: 'Nhanh, nhẹ · khuyên dùng', estimatedMb: 7 },
  { id: 'small', label: 'PP-OCRv6 Small', hint: 'Cân bằng tốc độ và độ chính xác', estimatedMb: 31 },
  { id: 'medium', label: 'PP-OCRv6 Medium', hint: 'Model lớn · ưu tiên độ chính xác', estimatedMb: 136 },
];

const settingsNavigation: Array<{ name: SettingsSection; label: string; caption: string; icon: typeof Monitor }> = [
  { name: 'General', label: 'Chung', caption: 'Ứng dụng', icon: Monitor },
  { name: 'Translation', label: 'Dịch', caption: 'Dịch vụ dịch', icon: Languages },
  { name: 'Speech Recognition', label: 'Nhận dạng lời thoại', caption: 'Bản chép lời', icon: AudioLines },
  { name: 'Voice / TTS', label: 'Giọng đọc', caption: 'Lồng tiếng', icon: Volume2 },
  { name: 'Advanced', label: 'Nâng cao', caption: 'Dung lượng và hỗ trợ', icon: SlidersHorizontal },
  { name: 'About', label: 'Giới thiệu', caption: 'Thông tin', icon: Info },
];

export default function SettingsView() {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const requestedSection = searchParams.section;
  const initialSection = typeof requestedSection === 'string' && settingsNavigation.some((item) => item.name === requestedSection)
    ? requestedSection as SettingsSection
    : 'General';
  const [section, setSection] = createSignal<SettingsSection>(initialSection);
  const [settings, setSettings] = createSignal<AppSettings>();
  const [geminiConfigured, setGeminiConfigured] = createSignal(false);
  const [cloudConfigured, setCloudConfigured] = createSignal(false);
  const [secret, setSecret] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const [settingsMessage, setSettingsMessage] = createSignal('');
  const [settingsError, setSettingsError] = createSignal(false);
  const [testBusy, setTestBusy] = createSignal(false);
  const [testMessage, setTestMessage] = createSignal('');
  const [testError, setTestError] = createSignal(false);
  const [asrEnvironment, setAsrEnvironment] = createSignal<AsrEnvironment>();
  const [asrEnvironmentError, setAsrEnvironmentError] = createSignal('');
  const [asrEnvironmentLoading, setAsrEnvironmentLoading] = createSignal(false);
  const [installingAsrModel, setInstallingAsrModel] = createSignal(false);
  const [removingAsrModel, setRemovingAsrModel] = createSignal(false);
  const [showAsrModelConfirm, setShowAsrModelConfirm] = createSignal(false);
  const [showRemoveAsrModelConfirm, setShowRemoveAsrModelConfirm] = createSignal(false);
  const [installingWhisperXAlignment, setInstallingWhisperXAlignment] = createSignal(false);
  const [ocrEnvironment, setOcrEnvironment] = createSignal<OcrEnvironment>();
  const [ocrEnvironmentError, setOcrEnvironmentError] = createSignal('');
  const [ocrEnvironmentLoading, setOcrEnvironmentLoading] = createSignal(false);
  const [installingOcrRuntime, setInstallingOcrRuntime] = createSignal(false);
  const [installingOcrModel, setInstallingOcrModel] = createSignal(false);
  const [removingOcrModel, setRemovingOcrModel] = createSignal(false);
  const [showOcrModelConfirm, setShowOcrModelConfirm] = createSignal(false);
  const [showIntelInstall, setShowIntelInstall] = createSignal(false);
  const [showRemoveOcrModelConfirm, setShowRemoveOcrModelConfirm] = createSignal(false);
  const [ocrModelProgress, setOcrModelProgress] = createSignal({
    progress: 0,
    downloadedBytes: 0,
    totalBytes: 7 * 1024 * 1024,
    message: 'Đang chuẩn bị tải model…',
  });
  const [textRemovalEnvironment, setTextRemovalEnvironment] = createSignal<TextRemovalEnvironment>();
  const [textRemovalEnvironmentLoading, setTextRemovalEnvironmentLoading] = createSignal(false);
  const [textRemovalEnvironmentError, setTextRemovalEnvironmentError] = createSignal('');
  const [installingSttnModel, setInstallingSttnModel] = createSignal(false);
  const [removingSttnModel, setRemovingSttnModel] = createSignal(false);
  const [showSttnModelConfirm, setShowSttnModelConfirm] = createSignal(false);
  const [showRemoveSttnModelConfirm, setShowRemoveSttnModelConfirm] = createSignal(false);
  const [sttnModelProgress, setSttnModelProgress] = createSignal({ progress: 0, downloadedBytes: 0, totalBytes: 66_252_587, message: 'Đang chuẩn bị tải model…' });
  const [ttsEnvironment, setTtsEnvironment] = createSignal<TtsEnvironment>();
  const [ttsEnvironmentError, setTtsEnvironmentError] = createSignal('');
  const [ttsEnvironmentLoading, setTtsEnvironmentLoading] = createSignal(false);
  const [showTtsModelConfirm, setShowTtsModelConfirm] = createSignal(false);
  const [installingTtsModel, setInstallingTtsModel] = createSignal(false);
  const [removingTtsModel, setRemovingTtsModel] = createSignal(false);
  const [dialogueEnvironment, setDialogueEnvironment] = createSignal<DialogueEnvironment>();
  const [dialogueEnvironmentLoading, setDialogueEnvironmentLoading] = createSignal(false);
  const [dialogueEnvironmentError, setDialogueEnvironmentError] = createSignal('');
  const [installingDialogueModel, setInstallingDialogueModel] = createSignal(false);
  const [removingDialogueModel, setRemovingDialogueModel] = createSignal(false);
  const [showRemoveDialogueModelConfirm, setShowRemoveDialogueModelConfirm] = createSignal(false);
  const [dialogueModelProgress, setDialogueModelProgress] = createSignal<DialogueModelProgress>({ progress: 0, downloadedBytes: 0, totalBytes: 446_680_129, message: 'Đang chuẩn bị tải model…' });
  const [cudaInstallTarget, setCudaInstallTarget] = createSignal<'asr' | 'ocr' | 'tts'>();
  const [cudaInstalling, setCudaInstalling] = createSignal(false);
  const [cudaInstallProgress, setCudaInstallProgress] = createSignal('');
  const [cudaInstallError, setCudaInstallError] = createSignal('');
  const [showRemoveTtsModelConfirm, setShowRemoveTtsModelConfirm] = createSignal(false);
  const [ttsModelProgress, setTtsModelProgress] = createSignal(0);
  const [ttsModelMessage, setTtsModelMessage] = createSignal('');
  const [previewBusy, setPreviewBusy] = createSignal(false);
  const [voicePreviewUrl, setVoicePreviewUrl] = createSignal('');
  const [showVoiceProfileDialog, setShowVoiceProfileDialog] = createSignal(false);
  const [voiceProfileName, setVoiceProfileName] = createSignal('');
  const [voiceProfileFile, setVoiceProfileFile] = createSignal('');
  const [voiceProfileConsent, setVoiceProfileConsent] = createSignal(false);
  const [voiceProfileBusy, setVoiceProfileBusy] = createSignal(false);
  const [storageUsage, setStorageUsage] = createSignal<StorageUsage>();
  const [storageBusy, setStorageBusy] = createSignal(false);
  const [storageError, setStorageError] = createSignal('');
  const [showClearInstallCacheConfirm, setShowClearInstallCacheConfirm] = createSignal(false);
  const [runtimeHealth, setRuntimeHealth] = createSignal<RuntimeHealth>();
  const [runtimeHealthError, setRuntimeHealthError] = createSignal('');
  const [diagnosticsBusy, setDiagnosticsBusy] = createSignal(false);
  let unlistenTtsModel: (() => void) | undefined;
  let unlistenOcrModel: (() => void) | undefined;
  let unlistenDialogueModel: (() => void) | undefined;
  let unlistenOptionalRuntime: (() => void) | undefined;
  let unlistenSttnModel: (() => void) | undefined;
  let asrEnvironmentRevision = 0;
  let ocrEnvironmentRevision = 0;
  const returnPath = () => {
    const value = searchParams.returnTo;
    return typeof value === 'string' && value.startsWith('/') && !value.startsWith('//') ? value : '/';
  };

  const showMessage = (message: unknown, failed: boolean) => {
    setSettingsError(failed);
    setSettingsMessage(message instanceof Error ? message.message : String(message));
  };
  const refreshStorage = async () => {
    setStorageBusy(true); setStorageError('');
    try { setStorageUsage(await storageService.usage()); }
    catch (error) { setStorageError(error instanceof Error ? error.message : String(error)); }
    finally { setStorageBusy(false); }
  };
  const openStorageFolder = async (openFolder: () => Promise<string>) => {
    setStorageError('');
    try { await openFolder(); }
    catch (error) { setStorageError(error instanceof Error ? error.message : String(error)); }
  };
  const clearInstallCache = async () => {
    setShowClearInstallCacheConfirm(false);
    setStorageBusy(true);
    setStorageError('');
    try {
      const bytes = await storageService.clearAiInstallationCache();
      showMessage(`Đã xóa ${formatBytes(bytes)} cache cài đặt.`, false);
      await refreshStorage();
    } catch (error) {
      setStorageError(error instanceof Error ? error.message : String(error));
    } finally { setStorageBusy(false); }
  };
  const refreshRuntimeHealth = async () => {
    setRuntimeHealthError('');
    try { setRuntimeHealth(await runtimeService.health()); }
    catch (error) { setRuntimeHealthError(error instanceof Error ? error.message : String(error)); }
  };
  const exportDiagnostics = async () => {
    setDiagnosticsBusy(true);
    try {
      const selected = await open({ directory: true, multiple: false, title: 'Chọn thư mục lưu diagnostics' });
      if (typeof selected !== 'string') return;
      const path = await runtimeService.exportDiagnostics(selected);
      showMessage(`Đã lưu gói chẩn đoán: ${path}`, false);
    } catch (error) { showMessage(error, true); }
    finally { setDiagnosticsBusy(false); }
  };
  onMount(() => {
    setAppStartupReady(true);
    void translationService.getSettings().then((result) => {
      setSettings(result.settings);
      setGeminiConfigured(result.geminiCredentialConfigured);
      setCloudConfigured(result.googleCloudCredentialConfigured);
      void refreshAsrEnvironment(result.settings.asr.model, result.settings.asr.language);
      void refreshOcrEnvironment(result.settings.asr.ocrModel);
      void refreshTextRemovalEnvironment();
    }).catch((error: unknown) => showMessage(error, true));
    void refreshTtsEnvironment();
    void refreshDialogueEnvironment();
    void refreshStorage();
    void refreshRuntimeHealth();
    void listen<{ progress?: number; message?: string }>('tts-model-progress', (event) => {
      setTtsModelProgress(event.payload.progress ?? 0);
      setTtsModelMessage(event.payload.message ?? 'Đang tải model…');
    }).then((unlisten) => { unlistenTtsModel = unlisten; }).catch(() => undefined);
    void listen<{ progress?: number; downloadedBytes?: number; totalBytes?: number; message?: string }>('ocr-model-progress', (event) => {
      setOcrModelProgress((value) => ({
        progress: Math.max(0, Math.min(1, event.payload.progress ?? value.progress)),
        downloadedBytes: Math.max(0, event.payload.downloadedBytes ?? value.downloadedBytes),
        totalBytes: Math.max(1, event.payload.totalBytes ?? value.totalBytes),
        message: event.payload.message ?? value.message,
      }));
    }).then((unlisten) => { unlistenOcrModel = unlisten; }).catch(() => undefined);
    void listen<{ progress?: number; downloadedBytes?: number; totalBytes?: number; message?: string }>('sttn-model-progress', (event) => {
      setSttnModelProgress((value) => ({
        progress: Math.max(0, Math.min(1, event.payload.progress ?? value.progress)),
        downloadedBytes: Math.max(0, event.payload.downloadedBytes ?? value.downloadedBytes),
        totalBytes: Math.max(1, event.payload.totalBytes ?? value.totalBytes),
        message: event.payload.message ?? value.message,
      }));
    }).then((unlisten) => { unlistenSttnModel = unlisten; }).catch(() => undefined);
    void dialogueService.onModelProgress(setDialogueModelProgress)
      .then((unlisten) => { unlistenDialogueModel = unlisten; }).catch(() => undefined);
    void optionalRuntimeService.onProgress(setCudaInstallProgress)
      .then((unlisten) => { unlistenOptionalRuntime = unlisten; }).catch(() => undefined);
  });
  onCleanup(() => { unlistenTtsModel?.(); unlistenOcrModel?.(); unlistenDialogueModel?.(); unlistenOptionalRuntime?.(); unlistenSttnModel?.(); });
  const refreshDialogueEnvironment = async () => {
    setDialogueEnvironmentLoading(true);
    setDialogueEnvironmentError('');
    try { setDialogueEnvironment(await dialogueService.environment()); }
    catch (error) { setDialogueEnvironmentError(error instanceof Error ? error.message : 'Không thể kiểm tra model tách thoại.'); }
    finally { setDialogueEnvironmentLoading(false); }
  };
  const refreshTtsEnvironment = async () => {
    setTtsEnvironmentLoading(true);
    setTtsEnvironmentError('');
    try { setTtsEnvironment(await ttsService.getEnvironment()); }
    catch (error) { setTtsEnvironmentError(error instanceof Error ? error.message : 'Không thể kiểm tra VieNeu-TTS.'); }
    finally { setTtsEnvironmentLoading(false); }
  };
  const refreshAsrEnvironment = async (model = settings()?.asr.model ?? 'small', language = settings()?.asr.language ?? 'auto') => {
    const revision = ++asrEnvironmentRevision;
    setAsrEnvironmentLoading(true);
    setAsrEnvironment(undefined);
    setAsrEnvironmentError('');
    try {
      const environment = await projectService.getAsrEnvironment(model, language, settings()?.asr.engine, settings()?.asr.device);
      if (revision === asrEnvironmentRevision) setAsrEnvironment(environment);
    } catch (error) {
      if (revision === asrEnvironmentRevision) setAsrEnvironmentError(error instanceof Error ? error.message : 'Không thể kiểm tra Speech Recognition.');
    } finally {
      if (revision === asrEnvironmentRevision) setAsrEnvironmentLoading(false);
    }
  };
  const refreshOcrEnvironment = async (model = settings()?.asr.ocrModel ?? 'tiny') => {
    const revision = ++ocrEnvironmentRevision;
    setOcrEnvironmentLoading(true);
    setOcrEnvironmentError('');
    try {
      const environment = await projectService.getOcrEnvironment(model);
      if (revision === ocrEnvironmentRevision) setOcrEnvironment(environment);
    } catch (error) {
      if (revision === ocrEnvironmentRevision) setOcrEnvironmentError(error instanceof Error ? error.message : 'Không thể kiểm tra RapidOCR.');
    } finally { if (revision === ocrEnvironmentRevision) setOcrEnvironmentLoading(false); }
  };
  const refreshTextRemovalEnvironment = async () => {
    setTextRemovalEnvironmentLoading(true);
    setTextRemovalEnvironmentError('');
    try { setTextRemovalEnvironment(await projectService.getTextRemovalEnvironment()); }
    catch (error) { setTextRemovalEnvironmentError(error instanceof Error ? error.message : 'Không thể kiểm tra STTN.'); }
    finally { setTextRemovalEnvironmentLoading(false); }
  };
  const rapidOcrReady = () => !!ocrEnvironment()?.workerInstalled && !!ocrEnvironment()?.ocrInstalled && !!ocrEnvironment()?.runtimeInstalled && !!ocrEnvironment()?.modelCached;
  const updateTranslation = (update: Partial<AppSettings['translation']>) => {
    setSettings((value) => value ? { ...value, translation: { ...value.translation, ...update } } : value);
  };
  const updateTts = (update: Partial<AppSettings['tts']>) => {
    setSettings((value) => value ? { ...value, tts: { ...value.tts, ...update } } : value);
  };
  const updateAsr = (update: Partial<AppSettings['asr']>) => {
    setSettings((value) => value ? { ...value, asr: { ...value.asr, ...update } } : value);
  };
  const chooseDevice = (target: 'asr' | 'ocr' | 'tts', device: 'cpu' | 'cuda') => {
    if (device === 'cpu') {
      if (target === 'tts') updateTts({ device });
      else updateAsr(target === 'ocr' ? { ocrDevice: device } : { device });
      return;
    }
    const ready = target === 'tts' ? !!ttsEnvironment()?.cudaAvailable && !!ttsEnvironment()?.modelCached
      : target === 'ocr' ? !!ocrEnvironment()?.cudaAvailable && !!ocrEnvironment()?.modelCached
      : !!asrEnvironment()?.cudaAvailable && !!asrEnvironment()?.modelCached && (settings()?.asr.engine !== 'whisperX' || !!asrEnvironment()?.whisperXInstalled && !!asrEnvironment()?.alignmentModelCached);
    if (ready) {
      if (target === 'tts') updateTts({ device });
      else updateAsr(target === 'ocr' ? { ocrDevice: device } : { device });
      return;
    }
    setCudaInstallError('');
    setCudaInstallTarget(target);
  };
  const installCudaFor = async () => {
    const target = cudaInstallTarget();
    const current = settings();
    if (!target || !current || cudaInstalling()) return;
    if (target === 'asr' && current.asr.engine === 'whisperX' && current.asr.language === 'auto') {
      setCudaInstallError('Chọn ngôn ngữ WhisperX trước để ứng dụng cài đúng model căn thời gian.');
      return;
    }
    setCudaInstalling(true);
    setCudaInstallError('');
    setCudaInstallProgress('Đang chuẩn bị tăng tốc NVIDIA…');
    try {
      await optionalRuntimeService.install(target === 'asr' && current.asr.engine === 'whisperX' ? 'whisperx' : 'acceleration', 'cuda');
      if (target === 'asr') {
        setCudaInstallProgress(`Đang chuẩn bị model nhận dạng ${current.asr.model}…`);
        await projectService.installAsrModel(current.asr.model);
        if (current.asr.engine === 'whisperX') {
          setCudaInstallProgress(`Đang chuẩn bị model căn thời gian ${current.asr.language.toUpperCase()}…`);
          await projectService.installWhisperXAlignment(current.asr.language);
        }
      } else if (target === 'ocr') {
        setCudaInstallProgress(`Đang chuẩn bị model OCR ${current.asr.ocrModel}…`);
        await projectService.installOcrModel(current.asr.ocrModel);
      } else {
        setCudaInstallProgress('Đang chuẩn bị model giọng đọc VieNeu…');
        await ttsService.installModel();
      }
      const next: AppSettings = target === 'tts' ? { ...current, tts: { ...current.tts, device: 'cuda' } }
        : { ...current, asr: { ...current.asr, [target === 'ocr' ? 'ocrDevice' : 'device']: 'cuda' } };
      const saved = await translationService.saveSettings(next);
      setSettings(saved.settings);
      await Promise.all([refreshAsrEnvironment(), refreshOcrEnvironment(), refreshTextRemovalEnvironment(), refreshTtsEnvironment(), refreshDialogueEnvironment(), refreshStorage()]);
      setCudaInstallTarget(undefined);
      showMessage('Đã cài thành phần NVIDIA và model cần thiết. Thiết bị CUDA đã được lưu.', false);
    } catch (error) {
      setCudaInstallError(error instanceof Error ? error.message : String(error));
    } finally {
      setCudaInstalling(false);
      setCudaInstallProgress('');
    }
  };
  const saveSettings = async () => {
    const value = settings();
    if (!value || busy()) return;
    setBusy(true);
    try {
      const result = await translationService.saveSettings(value);
      setSettings(result.settings);
      setGeminiConfigured(result.geminiCredentialConfigured);
      setCloudConfigured(result.googleCloudCredentialConfigured);
      await refreshOcrEnvironment(result.settings.asr.ocrModel);
      await refreshTextRemovalEnvironment();
      showMessage('Đã lưu cấu hình ứng dụng trên thiết bị.', false);
    } catch (error) { showMessage(error, true); }
    finally { setBusy(false); }
  };
  const saveCredential = async (): Promise<boolean> => {
    const provider = settings()?.translation.provider;
    if (!provider || provider === 'codexCli' || !secret().trim() || busy()) return false;
    setBusy(true);
    try {
      await translationService.setCredential(provider, secret());
      setSecret('');
      const refreshed = await translationService.getSettings();
      setGeminiConfigured(refreshed.geminiCredentialConfigured);
      setCloudConfigured(refreshed.googleCloudCredentialConfigured);
      showMessage('Đã lưu thông tin đăng nhập an toàn trên thiết bị.', false);
      return true;
    } catch (error) { showMessage(error, true); return false; }
    finally { setBusy(false); }
  };
  const removeCredential = async () => {
    const provider = settings()?.translation.provider;
    if (!provider || provider === 'codexCli' || busy()) return;
    setBusy(true);
    try {
      await translationService.deleteCredential(provider);
      if (provider === 'gemini') setGeminiConfigured(false);
      else setCloudConfigured(false);
      showMessage('Đã xóa thông tin đăng nhập khỏi thiết bị.', false);
    } catch (error) { showMessage(error, true); }
    finally { setBusy(false); }
  };
  const testConnection = async () => {
    const value = settings();
    if (!value || testBusy()) return;
    setTestBusy(true);
    setTestMessage('');
    setTestError(false);
    try {
      if (secret().trim() && value.translation.provider !== 'codexCli' && !await saveCredential()) {
        throw new Error(settingsMessage() || 'Không thể lưu thông tin đăng nhập an toàn.');
      }
      await translationService.saveSettings(value);
      const result = await translationService.testProvider(value.translation.provider);
      setTestMessage(result.message);
    } catch (error) {
      setTestError(true);
      setTestMessage(error instanceof Error ? error.message : 'Không thể kiểm tra provider.');
    } finally { setTestBusy(false); }
  };
  const installTtsModel = async () => {
    setShowTtsModelConfirm(false);
    setInstallingTtsModel(true);
    setTtsModelProgress(0);
    setTtsModelMessage('Chuẩn bị tải model VieNeu-TTS…');
    try {
      await ttsService.installModel();
      await refreshTtsEnvironment();
      setTtsModelProgress(1);
      setTtsModelMessage('Model VieNeu-TTS đã sẵn sàng.');
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Không thể tải model VieNeu-TTS.';
      setTtsEnvironmentError(message);
      setTtsModelMessage(message);
    } finally { setInstallingTtsModel(false); }
  };
  const removeTtsModel = async () => {
    setShowRemoveTtsModelConfirm(false);
    setRemovingTtsModel(true);
    try {
      await ttsService.removeModel();
      await refreshTtsEnvironment();
      showMessage('Đã gỡ model VieNeu-TTS khỏi thư mục model của ứng dụng.', false);
    } catch (error) { showMessage(error, true); }
    finally { setRemovingTtsModel(false); }
  };
  const installAsrModel = async () => {
    const currentSettings = settings();
    const model = currentSettings?.asr.model;
    if (!currentSettings || !model || installingAsrModel() || busy()) return;
    setShowAsrModelConfirm(false);
    setInstallingAsrModel(true);
    setAsrEnvironmentError('');
    try {
      const saved = await translationService.saveSettings(currentSettings);
      setSettings(saved.settings);
      await projectService.installAsrModel(model);
      await refreshAsrEnvironment(model, currentSettings.asr.language);
    } catch (error) {
      setAsrEnvironmentError(error instanceof Error ? error.message : 'Không thể cài model faster-whisper.');
    } finally { setInstallingAsrModel(false); }
  };
  const removeAsrModel = async () => {
    const model = settings()?.asr.model;
    if (!model || removingAsrModel()) return;
    setShowRemoveAsrModelConfirm(false);
    setRemovingAsrModel(true);
    try {
      await projectService.removeAsrModel(model);
      await refreshAsrEnvironment(model, settings()?.asr.language ?? 'auto');
      showMessage(`Đã gỡ model faster-whisper ${model}.`, false);
    } catch (error) { showMessage(error, true); }
    finally { setRemovingAsrModel(false); }
  };
  const installWhisperXAlignment = async (reportProgress: (message: string) => void) => {
    const value = settings();
    const language = value?.asr.language;
    if (!value || !language || language === 'auto') throw new Error('Chọn ngôn ngữ của video trong mục WhisperX trước khi cài.');
    if (installingWhisperXAlignment()) return;
    setInstallingWhisperXAlignment(true);
    setAsrEnvironmentError('');
    try {
      const saved = await translationService.saveSettings(value);
      setSettings(saved.settings);
      reportProgress(`Đang chuẩn bị model nhận dạng ${value.asr.model}…`);
      await projectService.installAsrModel(value.asr.model);
      reportProgress(`Đang tải model căn thời gian ${language.toUpperCase()}…`);
      await projectService.installWhisperXAlignment(language);
      await refreshAsrEnvironment(value.asr.model, language);
      await refreshStorage();
      showMessage(`Đã cài mô hình căn thời gian WhisperX cho ${language.toUpperCase()}.`, false);
    } catch (error) {
      setAsrEnvironmentError(error instanceof Error ? error.message : 'Không thể cài mô hình căn thời gian WhisperX.');
      throw error;
    } finally { setInstallingWhisperXAlignment(false); }
  };
  const installOcrModel = async () => {
    const value = settings();
    if (!value || installingOcrModel()) return;
    const profile = value.asr.ocrModel;
    const profileLabel = OCR_MODELS.find((item) => item.id === profile)?.label ?? 'PP-OCRv6 Tiny';
    setShowOcrModelConfirm(false);
    setInstallingOcrModel(true);
    setOcrModelProgress({
      progress: 0,
      downloadedBytes: 0,
      totalBytes: ocrEnvironment()?.estimatedDownloadBytes ?? 7 * 1024 * 1024,
      message: `Đang chuẩn bị tải ${profileLabel}…`,
    });
    setOcrEnvironmentError('');
    try {
      const saved = await translationService.saveSettings(value);
      setSettings(saved.settings);
      await projectService.installOcrModel(profile);
      await refreshOcrEnvironment(profile);
      await refreshTextRemovalEnvironment();
      await refreshStorage();
      setOcrModelProgress((progress) => ({ ...progress, progress: 1, downloadedBytes: ocrEnvironment()?.modelBytes ?? progress.downloadedBytes, message: `Đã cài ${profileLabel}.` }));
      showMessage(`Đã cài ${profileLabel} trong thư mục Models dùng chung.`, false);
    } catch (error) {
      setOcrEnvironmentError(error instanceof Error ? error.message : 'Không thể cài model OCR.');
    } finally { setInstallingOcrModel(false); }
  };
  const removeOcrModel = async () => {
    const profile = settings()?.asr.ocrModel ?? 'tiny';
    setShowRemoveOcrModelConfirm(false);
    setRemovingOcrModel(true);
    setOcrEnvironmentError('');
    try {
      const removedBytes = await projectService.removeOcrModel(profile);
      await refreshOcrEnvironment(profile);
      await refreshTextRemovalEnvironment();
      await refreshStorage();
      showMessage(`Đã xóa mô hình đọc chữ và giải phóng ${formatBytes(removedBytes)}. Dữ liệu dự án được giữ nguyên.`, false);
    } catch (error) {
      setOcrEnvironmentError(error instanceof Error ? error.message : 'Không thể gỡ model OCR.');
    } finally { setRemovingOcrModel(false); }
  };
  const installDialogueModel = async (propagateError = false) => {
    if (installingDialogueModel()) return;
    setInstallingDialogueModel(true);
    setDialogueEnvironmentError('');
    setDialogueModelProgress({ progress: 0, downloadedBytes: 0, totalBytes: dialogueEnvironment()?.estimatedDownloadBytes ?? 446_680_129, message: 'Đang kết nối tới Zenodo…' });
    try {
      await dialogueService.installModel();
      await refreshDialogueEnvironment();
      await refreshStorage();
      showMessage('Đã cài model BandIt v2 trong Models dùng chung.', false);
    } catch (error) {
      setDialogueEnvironmentError(error instanceof Error ? error.message : 'Không thể cài model BandIt v2.');
      if (propagateError) throw error;
    } finally { setInstallingDialogueModel(false); }
  };
  const importDialogueModelFromFile = async () => {
    if (installingDialogueModel()) return;
    try {
      const selected = await open({ multiple: false, directory: false, filters: [{ name: 'BandIt v2 checkpoint', extensions: ['ckpt'] }] });
      if (typeof selected !== 'string') return;
      setInstallingDialogueModel(true);
      setDialogueEnvironmentError('');
      await dialogueService.installModelFromFile(selected);
      await refreshDialogueEnvironment();
      await refreshStorage();
      showMessage('Đã xác minh và cài checkpoint BandIt v2.', false);
    } catch (error) {
      setDialogueEnvironmentError(error instanceof Error ? error.message : 'Không thể nhập checkpoint BandIt v2.');
    } finally { setInstallingDialogueModel(false); }
  };
  const removeDialogueModel = async () => {
    setShowRemoveDialogueModelConfirm(false);
    setRemovingDialogueModel(true);
    setDialogueEnvironmentError('');
    try {
      await dialogueService.removeModel();
      await refreshDialogueEnvironment();
      await refreshStorage();
      showMessage('Đã xóa checkpoint BandIt v2 khỏi thư mục Models.', false);
    } catch (error) {
      setDialogueEnvironmentError(error instanceof Error ? error.message : 'Không thể gỡ model BandIt v2.');
    } finally { setRemovingDialogueModel(false); }
  };
  const installOcrRuntime = async () => {
    if (installingOcrRuntime() || !import.meta.env.DEV) return;
    setInstallingOcrRuntime(true);
    setOcrEnvironmentError('');
    try {
      await projectService.installOcrRuntime();
      await refreshOcrEnvironment();
      await refreshTextRemovalEnvironment();
      showMessage('Bộ đọc chữ đã sẵn sàng. Tiếp theo, tải mô hình ở mục này.', false);
    } catch (error) {
      setOcrEnvironmentError(error instanceof Error ? error.message : 'Không thể cài bộ đọc chữ.');
    } finally { setInstallingOcrRuntime(false); }
  };
  const installSttnModel = async () => {
    if (installingSttnModel()) return;
    setShowSttnModelConfirm(false);
    setInstallingSttnModel(true);
    setSttnModelProgress({ progress: 0, downloadedBytes: 0, totalBytes: textRemovalEnvironment()?.sttnEstimatedDownloadBytes ?? 66_252_587, message: 'Đang kết nối…' });
    setTextRemovalEnvironmentError('');
    try {
      await projectService.installSttnTextRemovalModel();
      await Promise.all([refreshTextRemovalEnvironment(), refreshStorage()]);
      showMessage('Đã cài model STTN trong thư mục Models dùng chung.', false);
    } catch (error) {
      setTextRemovalEnvironmentError(error instanceof Error ? error.message : 'Không thể cài model STTN.');
    } finally { setInstallingSttnModel(false); }
  };
  const removeSttnModel = async () => {
    setShowRemoveSttnModelConfirm(false);
    setRemovingSttnModel(true);
    setTextRemovalEnvironmentError('');
    try {
      await projectService.removeSttnTextRemovalModel();
      await Promise.all([refreshTextRemovalEnvironment(), refreshStorage()]);
      showMessage('Đã gỡ model STTN khỏi thư mục Models dùng chung.', false);
    } catch (error) {
      setTextRemovalEnvironmentError(error instanceof Error ? error.message : 'Không thể gỡ model STTN.');
    } finally { setRemovingSttnModel(false); }
  };
  const previewTtsVoice = async () => {
    const voice = settings()?.tts.defaultVoice;
    if (!voice || previewBusy()) return;
    setPreviewBusy(true);
    setVoicePreviewUrl('');
    try { setVoicePreviewUrl(convertFileSrc(await ttsService.previewVoice(voice))); }
    catch (error) { showMessage(error, true); }
    finally { setPreviewBusy(false); }
  };
  const browseVoiceReference = async () => {
    try {
      const selected = await open({ multiple: false, directory: false, filters: [{ name: 'Audio reference', extensions: ['wav', 'mp3', 'm4a', 'aac', 'flac', 'ogg', 'opus'] }] });
      if (typeof selected === 'string') setVoiceProfileFile(selected);
    } catch (error) { showMessage(error, true); }
  };
  const addVoiceProfile = async () => {
    if (!voiceProfileName().trim() || !voiceProfileFile() || !voiceProfileConsent() || voiceProfileBusy()) return;
    setVoiceProfileBusy(true);
    try {
      await ttsService.addVoiceProfile(voiceProfileName(), voiceProfileFile(), true);
      const refreshed = await translationService.getSettings();
      setSettings(refreshed.settings);
      setVoiceProfileName(''); setVoiceProfileFile(''); setVoiceProfileConsent(false); setShowVoiceProfileDialog(false);
      showMessage('Đã thêm voice profile. Audio tham chiếu được chuẩn hóa và lưu cục bộ.', false);
    } catch (error) { showMessage(error, true); }
    finally { setVoiceProfileBusy(false); }
  };
  const removeVoiceProfile = async (profileId: string) => {
    if (voiceProfileBusy()) return;
    setVoiceProfileBusy(true);
    try {
      await ttsService.deleteVoiceProfile(profileId);
      const refreshed = await translationService.getSettings();
      setSettings(refreshed.settings);
      showMessage('Đã xóa voice profile khỏi thiết bị.', false);
    } catch (error) { showMessage(error, true); }
    finally { setVoiceProfileBusy(false); }
  };

  return (
    <main class="settings-page">
      <header class="settings-page-header">
        <div class="settings-page-title"><button class="settings-back-link" type="button" onClick={() => navigate(returnPath())}><ArrowLeft size={14} /> Quay lại</button><div class="section-label">TÙY CHỌN</div><h1>Cài đặt</h1><p>Thiết lập dịch và lồng tiếng cho các dự án trên thiết bị này.</p></div>
        <span class="settings-scope-pill"><LockKeyhole size={13} /> CÀI ĐẶT TOÀN CỤC</span>
      </header>
      <div class="settings-layout">
        <nav class="settings-nav" aria-label="Nhóm cài đặt">
          <span class="settings-nav-label">THIẾT LẬP</span>
          <For each={settingsNavigation}>{(item) => {
            const Icon = item.icon;
            return <button type="button" class="settings-nav-item" classList={{ selected: section() === item.name }} aria-current={section() === item.name ? 'page' : undefined} onClick={() => { setSection(item.name); if (item.name === 'Advanced') void refreshStorage(); }}>
              <span class="settings-nav-icon"><Icon size={17} /></span><span class="settings-nav-copy"><strong>{item.label}</strong><small>{item.caption}</small></span><Show when={section() === item.name}><ChevronRight size={15} class="settings-nav-chevron" /></Show>
            </button>;
          }}</For>
          <div class="settings-nav-note"><CircleHelp size={15} /><span>Dịch vụ dịch dùng chung; ngôn ngữ và phụ đề được lưu theo từng dự án.</span></div>
        </nav>

        <section class="settings-detail" aria-live="polite">
          <Show when={section() === 'General'}>
            <SettingsSectionHeader title="Chung" subtitle="Tùy chỉnh ứng dụng và cách hiển thị." />
            <div class="settings-card">
              <SettingRow icon={<Monitor size={17} />} title="Giao diện" description="Chủ đề màu của ứng dụng."><span class="setting-value"><span class="theme-swatch" /> Tối <Check size={14} /></span></SettingRow>
              <div class="settings-row-divider" />
              <SettingRow icon={<Globe2 size={17} />} title="Ngôn ngữ ứng dụng" description="Ngôn ngữ hiển thị trong giao diện."><span class="setting-value">Tiếng Việt <ChevronRight size={14} /></span></SettingRow>
              <div class="settings-row-divider" />
              <SettingRow icon={<Sparkles size={17} />} title="Phiên bản" description="JC AI desktop."><span class="version-chip">{runtimeHealth()?.appVersion ?? '…'} · Windows desktop</span></SettingRow>
            </div>
            <div class="settings-info-note"><LockKeyhole size={15} /><span>Dự án được lưu cục bộ trong thư mục dữ liệu ứng dụng của hệ điều hành.</span></div>
          </Show>

          <Show when={section() === 'Translation'}>
            <SettingsSectionHeader title="Dịch" subtitle="Chọn dịch vụ dịch dùng chung cho các dự án. Thông tin đăng nhập được lưu an toàn trên thiết bị." />
            <Show when={settings()} fallback={<div class="settings-card provider-settings-card">Đang đọc cài đặt dịch vụ dịch…</div>}>
              <div class="settings-card provider-settings-card">
                <label class="provider-field"><span>Dịch vụ đang dùng</span><select class="settings-select" value={settings()!.translation.provider} onChange={(event) => { setSecret(''); setTestMessage(''); setSettingsMessage(''); updateTranslation({ provider: event.currentTarget.value as TranslationProviderId }); }}>
                  <option value="gemini">Gemini</option><option value="googleCloud">Google Cloud Translation</option><option value="codexCli">Codex CLI</option>
                </select></label>
                <div class="provider-settings-divider" />
                <Show when={settings()!.translation.provider === 'gemini'}>
                  <div class="provider-config-grid">
                    <label class="provider-field provider-field-wide"><span>Gemini API key</span><input type="password" autocomplete="new-password" value={secret()} placeholder={geminiConfigured() ? 'Đã lưu an toàn · nhập để thay thế' : 'Nhập API key'} onInput={(event) => setSecret(event.currentTarget.value)} /></label>
                    <label class="provider-field"><span>Model ID</span><input value={settings()!.translation.gemini.modelId} onInput={(event) => updateTranslation({ gemini: { ...settings()!.translation.gemini, modelId: event.currentTarget.value } })} /></label>
                    <label class="provider-field"><span>Temperature</span><input type="number" min="0" max="2" step="0.1" value={settings()!.translation.gemini.temperature} onInput={(event) => updateTranslation({ gemini: { ...settings()!.translation.gemini, temperature: Number(event.currentTarget.value) } })} /></label>
                  </div>
                  <p class="provider-help">Gemini dịch nhiều đoạn cùng lúc để giữ ngữ cảnh. Khóa truy cập được lưu an toàn trên thiết bị.</p>
                </Show>
                <Show when={settings()!.translation.provider === 'googleCloud'}>
                  <div class="provider-config-grid">
                    <label class="provider-field provider-field-wide"><span>Google Cloud API key</span><input type="password" autocomplete="new-password" value={secret()} placeholder={cloudConfigured() ? 'Đã lưu an toàn · nhập để thay thế' : 'Nhập API key'} onInput={(event) => setSecret(event.currentTarget.value)} /></label>
                    <label class="provider-field"><span>Translation model</span><select class="settings-select" value={settings()!.translation.googleCloud.model} onChange={(event) => updateTranslation({ googleCloud: { model: event.currentTarget.value } })}><option value="nmt">NMT</option></select></label>
                  </div>
                  <p class="provider-help">Google Cloud dịch nhiều đoạn cùng lúc. Dịch vụ này không dùng ngữ cảnh giữa các đoạn.</p>
                </Show>
                <Show when={settings()!.translation.provider === 'codexCli'}>
                  <div class="provider-config-grid">
                    <label class="provider-field provider-field-wide"><span>Codex executable</span><input value={settings()!.translation.codexCli.executable} onInput={(event) => updateTranslation({ codexCli: { ...settings()!.translation.codexCli, executable: event.currentTarget.value } })} /></label>
                    <label class="provider-field"><span>Model ID (tùy chọn)</span><input value={settings()!.translation.codexCli.modelId ?? ''} placeholder="Dùng model mặc định" onInput={(event) => updateTranslation({ codexCli: { ...settings()!.translation.codexCli, modelId: event.currentTarget.value || null } })} /></label>
                    <label class="provider-field"><span>Timeout (giây)</span><input type="number" min="5" max="600" value={settings()!.translation.codexCli.timeoutSeconds} onInput={(event) => updateTranslation({ codexCli: { ...settings()!.translation.codexCli, timeoutSeconds: Number(event.currentTarget.value) } })} /></label>
                  </div>
                  <p class="provider-help">Để dùng Codex, hãy cài ứng dụng Codex và đăng nhập trên thiết bị này. JC AI chỉ gửi nội dung cần dịch.</p>
                </Show>
                <div class="provider-settings-divider" />
                <div class="provider-config-grid provider-common-config">
                  <label class="provider-field"><span>Segments / batch</span><input type="number" min="1" max="100" value={settings()!.translation.batchSize} onInput={(event) => updateTranslation({ batchSize: Number(event.currentTarget.value) })} /></label>
                  <label class="provider-field"><span>Ngữ cảnh mỗi phía</span><input type="number" min="0" max="8" value={settings()!.translation.contextSegments} onInput={(event) => updateTranslation({ contextSegments: Number(event.currentTarget.value) })} /></label>
                  <label class="provider-field"><span>Timeout HTTP (giây)</span><input type="number" min="5" max="600" value={settings()!.translation.timeoutSeconds} onInput={(event) => updateTranslation({ timeoutSeconds: Number(event.currentTarget.value) })} /></label>
                </div>
                <div class="provider-settings-divider" />
                <p class="provider-help">Prompt bổ sung phong cách và giọng dịch được lưu riêng trong Cài đặt dự án. Yêu cầu dịch mặc định luôn được áp dụng.</p>
                <div class="provider-action-row">
                  <Show when={settings()!.translation.provider !== 'codexCli'}>
                    <button class="button button-quiet provider-action-secondary" type="button" disabled={busy() || testBusy() || !secret().trim()} onClick={() => void saveCredential()}>{busy() ? 'Đang lưu…' : 'Lưu thông tin đăng nhập'}</button>
                    <button class="button button-quiet provider-action-secondary" type="button" disabled={busy() || testBusy() || !(settings()!.translation.provider === 'gemini' ? geminiConfigured() : cloudConfigured())} onClick={() => void removeCredential()}>Xóa thông tin đăng nhập</button>
                  </Show>
                  <button class="button button-quiet provider-action-secondary" type="button" disabled={busy() || testBusy()} onClick={() => void saveSettings()}>{busy() ? 'Đang lưu…' : 'Lưu cấu hình'}</button>
                  <button class="button button-primary" type="button" disabled={busy() || testBusy()} onClick={() => void testConnection()}>{testBusy() ? <><span class="spin status-spinner" /> Đang kiểm tra…</> : 'Kiểm tra kết nối'}</button>
                </div>
                <Show when={settingsMessage()}><div class="provider-feedback" classList={{ 'provider-feedback-error': settingsError() }} role={settingsError() ? 'alert' : 'status'}>{settingsMessage()}</div></Show>
                <Show when={testMessage()}><div class="provider-feedback" classList={{ 'provider-feedback-error': testError() }} role={testError() ? 'alert' : 'status'}>{testMessage()}</div></Show>
              </div>
            </Show>
          </Show>

          <Show when={section() === 'Speech Recognition'}>
            <SettingsSectionHeader title="Nhận dạng lời thoại" subtitle="Chọn cách tạo bản chép lời từ video." />
            <Show when={settings()} fallback={<div class="settings-card provider-settings-card">Đang đọc cấu hình Speech Recognition…</div>}>
              <div class="settings-card asr-settings-card">
                <div class="tts-model-status-row">
                  <div class="provider-card-icon"><AudioLines size={17} /></div>
                  <div class="provider-card-main"><span class="provider-eyebrow">CHẠY TRÊN THIẾT BỊ</span><h3>{settings()!.asr.engine === 'whisperX' ? 'WhisperX' : settings()!.asr.engine === 'openVinoWhisper' ? 'Whisper · Intel GPU' : settings()!.asr.engine === 'auto' ? 'Whisper · Tự động' : 'faster-whisper'}</h3>
                    <p>{asrEnvironmentLoading() ? 'Đang kiểm tra thành phần nhận dạng…' : !asrEnvironment()?.workerInstalled ? 'Bộ xử lý chưa sẵn sàng.' : settings()!.asr.engine === 'whisperX' && !asrEnvironment()?.whisperXInstalled ? 'Cài WhisperX bên dưới để dùng cách nhận dạng này.' : !asrEnvironment()?.modelCached ? `Model ${settings()!.asr.model} cho thiết bị đang chọn chưa được cài.` : settings()!.asr.engine === 'whisperX' ? settings()!.asr.language === 'auto' ? 'Chọn ngôn ngữ cụ thể để cài model căn chỉnh thời gian.' : asrEnvironment()?.alignmentModelCached ? `Model ASR ${settings()!.asr.model} và căn chỉnh ${settings()!.asr.language} đã sẵn sàng.` : `Model ASR ${settings()!.asr.model} đã có; chưa cài model căn chỉnh ${settings()!.asr.language}.` : `Model ${settings()!.asr.model} đã được cài trên thiết bị.`}</p>
                  </div>
                  <span class="provider-status-dot" classList={{ 'provider-status-dot-ready': !!asrEnvironment()?.modelCached && (settings()!.asr.engine !== 'whisperX' || (!!asrEnvironment()?.whisperXInstalled && !!asrEnvironment()?.alignmentModelCached)) }} />
                </div>
                <Show when={!asrEnvironmentLoading() && !asrEnvironment()?.workerInstalled}>
                  <div class="tts-install-instructions"><strong>Thành phần nhận dạng chưa sẵn sàng</strong><span>{settings()!.asr.device === 'openvino' || settings()!.asr.engine === 'openVinoWhisper' ? 'Cài tăng tốc Intel ở mục bên dưới, sau đó tải model bạn muốn sử dụng.' : 'Hãy cập nhật hoặc cài lại JC AI. Model chỉ tải khi bạn xác nhận.'}</span></div>
                </Show>
                <div class="provider-config-grid">
                  <label class="provider-field"><span>Cách nhận dạng</span><select class="settings-select" value={settings()!.asr.engine} onChange={(event) => { const engine = event.currentTarget.value as AppSettings['asr']['engine']; updateAsr({ engine, ...(engine === 'openVinoWhisper' ? { device: 'openvino' as const } : (engine === 'whisperX' || engine === 'fasterWhisper') && settings()!.asr.device === 'openvino' ? { device: 'cpu' as const } : {}) }); void refreshAsrEnvironment(); }}><option value="auto">Tự động · khuyên dùng</option><option value="openVinoWhisper">Whisper · Intel GPU</option><option value="fasterWhisper">faster-whisper · nhận dạng nhanh</option><option value="whisperX">WhisperX · căn chính xác thời gian</option></select></label>
                  <label class="provider-field"><span>Model ASR</span><select class="settings-select" value={settings()!.asr.model} onChange={(event) => { const model = event.currentTarget.value as AppSettings['asr']['model']; updateAsr({ model }); void refreshAsrEnvironment(model, settings()?.asr.language ?? 'auto'); }}><option value="tiny">tiny</option><option value="base">base</option><option value="small">small · khuyến nghị</option><option value="medium" disabled={settings()!.asr.device === 'openvino' || settings()!.asr.engine === 'openVinoWhisper'}>medium</option><option value="large-v3" disabled={settings()!.asr.device === 'openvino' || settings()!.asr.engine === 'openVinoWhisper'}>large-v3</option></select></label>
                  <label class="provider-field"><span>Thiết bị</span><select class="settings-select" value={settings()!.asr.device} onChange={(event) => {
  const device = event.currentTarget.value as AppSettings['asr']['device'];
  if (device === 'cuda') chooseDevice('asr', 'cuda');
  else updateAsr({ device, ...(device === 'openvino' && settings()!.asr.engine !== 'openVinoWhisper' ? { engine: 'auto' as const } : {}) });
  void refreshAsrEnvironment();
}}>
<option value="auto">Tự động</option><option value="cpu" disabled={settings()!.asr.engine === 'openVinoWhisper'}>CPU · INT8</option>
<option value="cuda" disabled={settings()!.asr.engine === 'openVinoWhisper'}>NVIDIA · CUDA</option><option value="openvino" disabled={settings()!.asr.engine === 'whisperX'}>Intel GPU · OpenVINO</option>
</select></label>
                  <Show when={settings()!.asr.engine !== 'whisperX'}><label class="provider-field"><span>Ngôn ngữ</span><select class="settings-select" value={settings()!.asr.language} onChange={(event) => { const language = event.currentTarget.value as AppSettings['asr']['language']; updateAsr({ language }); void refreshAsrEnvironment(settings()?.asr.model ?? 'small', language); }}><option value="auto">Tự nhận diện</option><option value="en">English</option><option value="vi">Tiếng Việt</option><option value="ja">日本語</option><option value="ko">한국어</option><option value="zh">中文</option><option value="fr">Français</option><option value="de">Deutsch</option><option value="es">Español</option></select></label></Show>
                </div>
                <Show when={settings()!.asr.engine === 'whisperX'}>
                  <div class="settings-info-note"><AudioLines size={15} /><span>WhisperX dùng mô hình nhận dạng đã cài, rồi căn lại thời gian theo ngôn ngữ. Hãy chọn ngôn ngữ cụ thể để dùng tính năng này.</span></div>
                  <OptionalAiRuntimeCard
                    embedded
                    preferredProfile={settings()!.asr.device === 'cuda' ? 'cuda' : 'cpu'}
                    feature="whisperx"
                    configuration={<div class="provider-config-grid"><label class="provider-field"><span>Ngôn ngữ để căn thời gian</span><select class="settings-select" value={settings()!.asr.language} onChange={(event) => { const language = event.currentTarget.value as AppSettings['asr']['language']; updateAsr({ language }); void refreshAsrEnvironment(settings()?.asr.model ?? 'small', language); }}><option value="auto">Chọn ngôn ngữ trước khi cài</option><option value="en">English</option><option value="vi">Tiếng Việt</option><option value="ja">日本語</option><option value="ko">한국어</option><option value="zh">中文</option><option value="fr">Français</option><option value="de">Deutsch</option><option value="es">Español</option></select></label></div>}
                    available={asrEnvironment()?.whisperXInstalled}
                    modelReady={!!asrEnvironment()?.modelCached && !!asrEnvironment()?.alignmentModelCached}
                    modelDescription={`model nhận dạng ${settings()!.asr.model} (khoảng ${estimatedModelSize(settings()!.asr.model)}) và model căn thời gian ${settings()!.asr.language.toUpperCase()} (dung lượng tùy ngôn ngữ)`}
                    installDisabledReason={asrEnvironmentLoading() ? 'Đang kiểm tra các model đã cài…' : settings()!.asr.language === 'auto' ? 'Chọn ngôn ngữ của video ở mục WhisperX để cài đúng model căn thời gian.' : undefined}
                    installModels={installWhisperXAlignment}
                    onInstalled={() => { void Promise.all([refreshAsrEnvironment(), refreshStorage()]); }}
                  />
                  <Show when={asrEnvironment()?.whisperXInstalled && asrEnvironment()?.alignmentModelCached}><span class="version-chip">Căn thời gian · {settings()!.asr.language} · Đã cài</span></Show>
                </Show>
                <div class="provider-help" role="status">Đang dùng: {asrEnvironment()?.resolvedLabel ?? 'Chưa sẵn sàng'} · CPU {asrEnvironment()?.backends?.cpu ? 'sẵn sàng' : 'chưa sẵn sàng'} · NVIDIA {asrEnvironment()?.cudaAvailable ? 'sẵn sàng' : 'chưa sẵn sàng'} · Intel {asrEnvironment()?.intelGpuAvailable ? 'sẵn sàng' : 'chưa sẵn sàng'}</div>
                <Show when={asrEnvironment()?.selectionError}><p class="asr-config-warning" role="alert">{asrEnvironment()?.selectionError}</p></Show>
                <Show when={settings()!.asr.engine !== 'whisperX'}><IntelWhisperCard model={settings()!.asr.model} environment={asrEnvironment()} onChanged={async () => { await refreshAsrEnvironment(); await refreshStorage(); }} /></Show>
                <p class="provider-help">Audio được trộn về mono và nhận dạng một lượt. Chọn model lớn hơn nếu cần cải thiện khả năng nhận diện lời thoại nhanh hoặc khó nghe; WhisperX giúp căn thời gian chính xác hơn.</p>
                <Show when={settings()!.asr.device === 'cuda' && asrEnvironment()?.workerInstalled && !asrEnvironment()?.cudaAvailable}><p class="asr-config-warning" role="alert">CUDA chưa sẵn sàng. Chọn lại NVIDIA trong mục Thiết bị để cài, hoặc dùng CPU.</p></Show>
                <Show when={asrEnvironmentError()}><div class="provider-feedback provider-feedback-error" role="alert">{asrEnvironmentError()}</div></Show>
                <Show when={installingAsrModel() || installingWhisperXAlignment()}><div class="tts-install-progress" role="status"><div><span>{installingWhisperXAlignment() ? `Đang cài WhisperX aligner ${settings()!.asr.language}…` : `Đang cài model ${settings()!.asr.model}…`} Giữ ứng dụng mở cho đến khi hoàn tất.</span><span><span class="spin status-spinner" /></span></div><div class="job-progress-track"><span class="job-progress-indeterminate" /></div></div></Show>
                <div class="provider-action-row">
                  <button class="button button-quiet provider-action-secondary" type="button" disabled={busy() || installingAsrModel() || installingWhisperXAlignment()} onClick={() => void saveSettings()}>{busy() ? 'Đang lưu…' : 'Lưu cấu hình'}</button>
                  <button class="button button-quiet provider-action-secondary" type="button" disabled={asrEnvironmentLoading() || installingAsrModel() || installingWhisperXAlignment()} onClick={() => void refreshAsrEnvironment()}>{asrEnvironmentLoading() ? 'Đang kiểm tra…' : 'Kiểm tra lại'}</button>
                  <Show when={settings()!.asr.engine !== 'whisperX' && settings()!.asr.device !== 'openvino' && settings()!.asr.engine !== 'openVinoWhisper' && asrEnvironment()?.workerInstalled && !asrEnvironment()?.ctranslateModelCached}>
                    <button class="button button-primary" type="button" disabled={busy() || installingAsrModel() || installingWhisperXAlignment() || asrEnvironmentLoading()} onClick={() => setShowAsrModelConfirm(true)}>{installingAsrModel() ? 'Đang cài model…' : `Cài model ${settings()!.asr.model}`}</button>
                  </Show>
                  <Show when={asrEnvironment()?.ctranslateModelCached}><span class="version-chip">CPU/NVIDIA đã cài · {settings()!.asr.model}</span><button class="button button-quiet" type="button" disabled={removingAsrModel() || installingAsrModel() || installingWhisperXAlignment()} onClick={() => setShowRemoveAsrModelConfirm(true)}>{removingAsrModel() ? 'Đang gỡ…' : 'Gỡ model'}</button></Show>
                </div>
              </div>
            </Show>
            <div class="settings-card ocr-settings-card">
              <div class="ocr-card-heading">
                <div class="provider-card-icon"><Sparkles size={18} /></div>
                <div class="ocr-card-title"><span class="provider-eyebrow">ĐỌC CHỮ TRÊN VIDEO</span><h3>Đọc phụ đề trong hình</h3><p>RapidOCR · {settings()!.asr.ocrDevice === 'auto' ? `Tự động · ${ocrEnvironment()?.resolvedLabel ?? 'Đang kiểm tra…'}` : settings()!.asr.ocrDevice === 'openvino' ? 'Intel GPU' : settings()!.asr.ocrDevice === 'cuda' ? 'NVIDIA' : 'CPU'}</p></div>
                <span class="ocr-overall-status" classList={{ ready: !!ocrEnvironment()?.ocrInstalled && !!ocrEnvironment()?.runtimeInstalled && !!ocrEnvironment()?.modelCached, loading: ocrEnvironmentLoading() || installingOcrRuntime() || installingOcrModel() || removingOcrModel() }}>
                  <span class="provider-status-dot" />{ocrEnvironmentLoading() ? 'Đang kiểm tra' : removingOcrModel() ? 'Đang gỡ model' : ocrEnvironment()?.modelCached ? 'Sẵn sàng' : ocrEnvironment()?.ocrInstalled && ocrEnvironment()?.runtimeInstalled ? 'Cần model' : 'Chưa cài'}
                </span>
              </div>

              <div class="provider-config-grid">
                <label class="provider-field"><span>Model OCR</span><select class="settings-select" value={settings()!.asr.ocrModel} onChange={(event) => { const model = event.currentTarget.value as AppSettings['asr']['ocrModel']; updateAsr({ ocrModel: model }); void refreshOcrEnvironment(model); }}>
                  <For each={OCR_MODELS}>{(model) => <option value={model.id}>{model.label} · khoảng {model.estimatedMb} MB</option>}</For>
                </select></label>
                <label class="provider-field"><span>Thiết bị</span><select class="settings-select" value={settings()!.asr.ocrDevice} onChange={(event) => {
                  const device = event.currentTarget.value as AppSettings['asr']['ocrDevice'];
                  if (device === 'auto') updateAsr({ ocrDevice: device });
                  else if (device === 'openvino') {
                    if (ocrEnvironment()?.backends?.openvino?.available) updateAsr({ ocrDevice: device });
                    else setShowIntelInstall(true);
                  } else chooseDevice('ocr', device);
                  event.currentTarget.value = settings()!.asr.ocrDevice;
                }}>
                  <option value="auto">Tự động · khuyên dùng</option>
                  <option value="cpu">CPU</option><option value="cuda">NVIDIA · CUDA{ocrEnvironment()?.cudaAvailable ? '' : ' · cài khi chọn'}</option>
                  <option value="openvino">Intel GPU · OpenVINO{ocrEnvironment()?.backends?.openvino?.available ? '' : ' · cài khi chọn'}</option>
                </select></label>
              </div>
              <Show when={!ocrEnvironmentLoading() && !ocrEnvironment()?.cudaAvailable}>
                <p class="provider-help">Chọn NVIDIA trong mục Thiết bị để cài tăng tốc và model OCR đang chọn trong một lượt.</p>
              </Show>

              <div class="ocr-component-grid" aria-label="Thành phần OCR">
                <div class="ocr-component"><span>BỘ ĐỌC CHỮ</span><strong>RapidOCR</strong><small>{ocrEnvironment()?.runtimeInstalled ? `Tự động ưu tiên: ${ocrEnvironment()?.resolvedLabel ?? 'CPU'}` : 'Cần cài thành phần bổ sung'}</small></div>
                <div class="ocr-component"><span>MODEL ĐANG CHỌN</span><strong>{OCR_MODELS.find((model) => model.id === settings()!.asr.ocrModel)?.label}</strong><small>{OCR_MODELS.find((model) => model.id === settings()!.asr.ocrModel)?.hint}</small></div>
                <div class="ocr-component"><span>DUNG LƯỢNG MODEL</span><strong>{ocrEnvironment()?.modelCached ? formatBytes(ocrEnvironment()!.modelBytes) : `Khoảng ${formatBytes(ocrEnvironment()?.estimatedDownloadBytes ?? 7 * 1024 * 1024)}`}</strong><small>{ocrEnvironment()?.modelCached ? 'Đã có trong Models' : 'Tải một lần, dùng offline'}</small></div>
              </div>

              <p class="ocr-card-help">Dùng cho phụ đề nằm sẵn trong hình. Chọn chế độ đọc chữ trong Cài đặt dự án, rồi kéo chọn vùng phụ đề trên video. Tốc độ đọc được chọn riêng cho từng dự án.</p>
              <Show when={settings()!.asr.ocrDevice === 'cuda' && !ocrEnvironment()?.cudaAvailable}><p class="asr-config-warning" role="alert">Tăng tốc NVIDIA chưa sẵn sàng. Chọn lại NVIDIA trong mục Thiết bị để cài, hoặc dùng CPU.</p></Show>
              <Show when={settings()!.asr.ocrDevice === 'openvino' && !ocrEnvironment()?.backends?.openvino?.available}><p class="asr-config-warning" role="alert">{ocrEnvironment()?.backends?.openvino?.message ?? 'Tăng tốc Intel chưa sẵn sàng. Cài tăng tốc hoặc chọn Tự động/CPU.'}</p></Show>
              <IntelOcrRuntimeCard environment={ocrEnvironment()} confirm={showIntelInstall()} onConfirmChange={setShowIntelInstall}
                onInstalled={async () => {
                  const environment = await projectService.getOcrEnvironment(settings()!.asr.ocrModel);
                  setOcrEnvironment(environment);
                  await refreshStorage();
                  if (environment.backends?.openvino?.available) {
                    updateAsr({ ocrDevice: 'openvino' });
                    await saveSettings();
                  }
                }} />

              <Show when={!ocrEnvironmentLoading() && ocrEnvironment()?.workerInstalled && (!ocrEnvironment()?.ocrInstalled || !ocrEnvironment()?.runtimeInstalled)}>
                <div class="ocr-install-callout">
                  <div class="ocr-install-copy"><strong>Thiếu thành phần đọc chữ</strong><span>Hãy cập nhật JC AI để khôi phục bộ đọc chữ cơ bản.</span></div>
                  <Show when={import.meta.env.DEV}><button class="button button-primary" type="button" disabled={installingOcrRuntime()} onClick={() => void installOcrRuntime()}>{installingOcrRuntime() ? 'Đang cài…' : 'Cài thành phần đọc chữ'}</button></Show>
                </div>
              </Show>

              <Show when={ocrEnvironment()?.ocrInstalled && ocrEnvironment()?.runtimeInstalled && !ocrEnvironment()?.modelCached && !ocrEnvironmentLoading()}>
                <div class="ocr-install-callout ocr-install-callout-ready">
                  <div class="ocr-install-copy"><strong>Bộ đọc chữ đã sẵn sàng</strong><span>Tải {ocrEnvironment()?.model ?? 'PP-OCRv6 Tiny'} · khoảng {formatBytes(ocrEnvironment()?.estimatedDownloadBytes ?? 7 * 1024 * 1024)}. Sau khi cài có thể dùng không cần Internet.</span></div>
                  <button class="button button-primary" type="button" disabled={installingOcrModel() || removingOcrModel()} onClick={() => setShowOcrModelConfirm(true)}>{installingOcrModel() ? 'Đang cài model…' : 'Tải model OCR'}</button>
                </div>
              </Show>

              <Show when={installingOcrRuntime() || installingOcrModel()}>
                <div class="ocr-install-progress" role="status" aria-live="polite">
                    <div class="ocr-progress-copy"><span class="spin status-spinner" /><span><strong>{installingOcrRuntime() ? 'Đang cài bộ đọc chữ' : `Đang tải ${ocrEnvironment()?.model ?? 'mô hình đọc chữ'}`}</strong><small>{installingOcrRuntime() ? 'Đang chuẩn bị thành phần đọc chữ…' : `${ocrModelProgress().message} · ${formatBytes(ocrModelProgress().downloadedBytes)} / khoảng ${formatBytes(ocrModelProgress().totalBytes)}`}</small></span><strong>{installingOcrRuntime() ? '' : `${Math.round(ocrModelProgress().progress * 100)}%`}</strong></div>
                  <div class="job-progress-track" role={installingOcrRuntime() ? undefined : 'progressbar'} aria-valuemin={installingOcrRuntime() ? undefined : 0} aria-valuemax={installingOcrRuntime() ? undefined : 100} aria-valuenow={installingOcrRuntime() ? undefined : Math.round(ocrModelProgress().progress * 100)}><span class={installingOcrRuntime() ? 'job-progress-indeterminate' : ''} style={installingOcrRuntime() ? undefined : { width: `${Math.max(2, ocrModelProgress().progress * 100)}%` }} /></div>
                </div>
              </Show>
              <Show when={ocrEnvironmentError()}><div class="provider-feedback provider-feedback-error" role="alert">{ocrEnvironmentError()}</div></Show>

              <div class="ocr-footer">
                <span class="ocr-footer-path">{storageUsage()?.ocrDirectory ?? 'Thư mục model dùng chung của ứng dụng'}</span>
                <button class="button button-quiet provider-action-secondary" type="button" disabled={busy() || installingOcrRuntime() || installingOcrModel() || removingOcrModel()} onClick={() => void saveSettings()}>{busy() ? 'Đang lưu…' : 'Lưu cấu hình OCR'}</button>
                <button class="button button-quiet provider-action-secondary" type="button" disabled={ocrEnvironmentLoading() || installingOcrRuntime() || installingOcrModel() || removingOcrModel()} onClick={() => void refreshOcrEnvironment(settings()!.asr.ocrModel)}>{ocrEnvironmentLoading() ? 'Đang kiểm tra…' : 'Kiểm tra lại'}</button>
                <Show when={ocrEnvironment()?.modelCached}><span class="version-chip">Đã cài · {formatBytes(ocrEnvironment()!.modelBytes)}</span><button class="button button-danger provider-action-secondary" type="button" disabled={installingOcrModel() || removingOcrModel()} onClick={() => setShowRemoveOcrModelConfirm(true)}>{removingOcrModel() ? 'Đang gỡ…' : `Gỡ ${settings()!.asr.ocrModel}`}</button></Show>
              </div>
            </div>
            <div class="settings-card ocr-settings-card">
              <section class="ocr-ai-removal-section" aria-labelledby="ocr-ai-removal-title">
                <div class="ocr-ai-removal-heading">
                  <div><span class="provider-eyebrow">PHỤC HỒI NỀN SAU PHỤ ĐỀ</span><h4 id="ocr-ai-removal-title">AI Remove · STTN</h4>
                    <p>STTN dùng các khung hình lân cận để phục hồi nền. RapidOCR chỉ dùng riêng khi cần phát hiện chữ trong video.</p></div>
                  <button class="button button-quiet provider-action-secondary" type="button" disabled={textRemovalEnvironmentLoading() || installingSttnModel() || removingSttnModel()} onClick={() => void refreshTextRemovalEnvironment()}>{textRemovalEnvironmentLoading() ? 'Đang kiểm tra…' : 'Kiểm tra lại'}</button>
                </div>
                <Show when={!rapidOcrReady()}>
                  <div class="ocr-ai-prerequisite" role="status"><Sparkles size={15} /><span><strong>RapidOCR chưa sẵn sàng</strong><small>Cài RapidOCR riêng để dò thời điểm có chữ. STTN được cài độc lập.</small></span></div>
                </Show>
                <div class="ocr-ai-addon-grid">
                  <div class="ocr-ai-addon-card ocr-ai-sttn-card">
                    <div class="ocr-ai-addon-title"><div><strong>STTN · Xóa chữ theo thời gian</strong><small>Dùng các khung hình lân cận · bộ xử lý và trọng số được cài riêng</small></div>
                      <span class="version-chip">{textRemovalEnvironmentLoading() ? 'Đang kiểm tra' : textRemovalEnvironment()?.sttnModelInstalled ? `Model · ${formatBytes(textRemovalEnvironment()!.sttnModelBytes)}` : `Model · khoảng ${formatBytes(textRemovalEnvironment()?.sttnEstimatedDownloadBytes ?? 66_252_587)}`}</span>
                    </div>
                    <OptionalAiRuntimeCard embedded feature="sttn" available={textRemovalEnvironment()?.sttnRuntimeInstalled} preferredProfile={textRemovalEnvironment()?.sttnCudaAvailable ? 'cuda' : undefined} onInstalled={() => { void Promise.all([refreshTextRemovalEnvironment(), refreshStorage()]); }} />
                    <div class="ocr-ai-addon-actions">
                      <Show when={!textRemovalEnvironment()?.sttnModelInstalled}><button class="button button-primary" type="button" disabled={textRemovalEnvironmentLoading() || installingSttnModel()} onClick={() => setShowSttnModelConfirm(true)}><Show when={installingSttnModel()} fallback={<Sparkles size={14} />}><LoaderCircle size={14} class="spin" /></Show>{installingSttnModel() ? 'Đang cài model…' : 'Cài model STTN'}</button></Show>
                      <Show when={textRemovalEnvironment()?.sttnModelInstalled}><span class="ocr-ai-ready-label"><Check size={13} /> Model đã cài</span><button class="button button-danger provider-action-secondary" type="button" disabled={removingSttnModel() || installingSttnModel()} onClick={() => setShowRemoveSttnModelConfirm(true)}><Trash2 size={13} />{removingSttnModel() ? 'Đang gỡ…' : 'Gỡ model'}</button></Show>
                    </div>
                    <Show when={installingSttnModel()}><div class="ocr-install-progress" role="status" aria-live="polite"><div class="ocr-progress-copy"><span class="spin status-spinner" /><span><strong>Đang cài model STTN</strong><small>{sttnModelProgress().message} · {formatBytes(sttnModelProgress().downloadedBytes)} / {formatBytes(sttnModelProgress().totalBytes)}</small></span><strong>{Math.round(sttnModelProgress().progress * 100)}%</strong></div><div class="job-progress-track" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(sttnModelProgress().progress * 100)}><span style={{ width: `${Math.max(2, sttnModelProgress().progress * 100)}%` }} /></div></div></Show>
                  </div>
                </div>
                <Show when={textRemovalEnvironmentError()}><div class="provider-feedback provider-feedback-error" role="alert">{textRemovalEnvironmentError()}</div></Show>
              </section>
            </div>
            <div class="settings-card ocr-settings-card dialogue-settings-card">
              <div class="ocr-card-heading">
                <div class="provider-card-icon"><AudioLines size={18} /></div>
                <div class="ocr-card-title"><span class="provider-eyebrow">XỬ LÝ ÂM THANH</span><h3>Tách thoại gốc</h3><p>BandIt v2 · tách thoại khỏi nhạc và hiệu ứng âm thanh</p></div>
                <span class="ocr-overall-status" classList={{ ready: !!dialogueEnvironment()?.modelCached && !!dialogueEnvironment()?.runtimeInstalled, loading: dialogueEnvironmentLoading() || installingDialogueModel() || removingDialogueModel() }}>
                  <span class="provider-status-dot" />{dialogueEnvironmentLoading() ? 'Đang kiểm tra' : removingDialogueModel() ? 'Đang gỡ mô hình' : dialogueEnvironment()?.modelCached && dialogueEnvironment()?.runtimeInstalled ? 'Sẵn sàng' : dialogueEnvironment()?.modelCached ? 'Cần cài thành phần xử lý' : 'Chưa cài mô hình'}
                </span>
              </div>
              <div class="ocr-component-grid" aria-label="Thành phần tách thoại">
                <div class="ocr-component"><span>BỘ XỬ LÝ</span><strong>{dialogueEnvironment()?.runtimeInstalled ? 'Đã cài' : 'Chưa cài'}</strong><small>{dialogueEnvironment()?.runtimeInstalled ? dialogueEnvironment()?.cudaAvailable ? 'Có thể tăng tốc bằng NVIDIA' : 'Chạy trên CPU' : 'Cài BandIt bên dưới'}</small></div>
                <div class="ocr-component"><span>MODEL</span><strong>BandIt v2 · Multi</strong><small>{dialogueEnvironment()?.modelCached ? `${formatBytes(dialogueEnvironment()!.modelBytes)} · đã cài` : 'Chưa tải về'}</small></div>
                <div class="ocr-component"><span>DUNG LƯỢNG</span><strong>{formatBytes(dialogueEnvironment()?.estimatedDownloadBytes ?? 446_680_129)}</strong><small>{dialogueEnvironment()?.modelCached ? 'Đã lưu trong Models dùng chung' : 'Checkpoint tải một lần, dùng offline'}</small></div>
              </div>
              <p class="ocr-card-help">Khi xuất video, ứng dụng tách lời thoại để giảm riêng tiếng nói, giữ nhạc và hiệu ứng gốc. Xử lý có thể mất thêm thời gian, nhất là trên CPU.</p>
              <Show when={!dialogueEnvironment()?.runtimeInstalled}><p class="asr-config-warning" role="status">Chọn thiết bị ngay trong mục này để cài bộ xử lý cùng model BandIt.</p></Show>
              <Show when={installingDialogueModel()}>
                <div class="ocr-install-progress" role="status" aria-live="polite"><div class="ocr-progress-copy"><span class="spin status-spinner" /><span><strong>Đang tải model BandIt v2</strong><small>{dialogueModelProgress().message} · {formatBytes(dialogueModelProgress().downloadedBytes)} / {formatBytes(dialogueModelProgress().totalBytes)}</small></span><strong>{Math.round(dialogueModelProgress().progress * 100)}%</strong></div><div class="job-progress-track" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(dialogueModelProgress().progress * 100)}><span style={{ width: `${Math.max(2, dialogueModelProgress().progress * 100)}%` }} /></div></div>
              </Show>
              <Show when={dialogueEnvironmentError()}><div class="provider-feedback provider-feedback-error" role="alert">{dialogueEnvironmentError()}</div></Show>
              <Show when={!dialogueEnvironment()?.modelCached}>
                <p class="provider-help">Nếu Zenodo chặn tải trong ứng dụng, tải <a href="https://zenodo.org/records/12701995/files/checkpoint-multi.ckpt?download=1" target="_blank" rel="noopener noreferrer">checkpoint-multi.ckpt từ Zenodo</a> bằng trình duyệt rồi chọn file bên dưới. Ứng dụng kiểm tra SHA-256 trước khi cài.</p>
                <button class="button button-quiet provider-action-secondary" type="button" disabled={installingDialogueModel() || dialogueEnvironmentLoading()} onClick={() => void importDialogueModelFromFile()}>Cài từ file đã tải</button>
              </Show>
              <div class="ocr-footer">
                <span class="ocr-footer-path" title={storageUsage()?.dialogueSeparationDirectory}>{storageUsage()?.dialogueSeparationDirectory ?? 'Thư mục model dùng chung của ứng dụng'}</span>
                <button class="button button-quiet provider-action-secondary" type="button" disabled={dialogueEnvironmentLoading() || installingDialogueModel() || removingDialogueModel()} onClick={() => void refreshDialogueEnvironment()}>{dialogueEnvironmentLoading() ? 'Đang kiểm tra…' : 'Kiểm tra lại'}</button>
                <button class="button button-quiet provider-action-secondary" type="button" onClick={() => void openStorageFolder(() => storageService.openModelFolder('dialogueSeparation'))}><FolderOpen size={13} /> Mở Models</button>
                <Show when={dialogueEnvironment()?.modelCached}>
                  <span class="version-chip">Đã cài · {formatBytes(dialogueEnvironment()!.modelBytes)}</span><button class="button button-danger provider-action-secondary" type="button" disabled={installingDialogueModel() || removingDialogueModel()} onClick={() => setShowRemoveDialogueModelConfirm(true)}>{removingDialogueModel() ? 'Đang gỡ…' : 'Xóa model và file'}</button>
                </Show>
              </div>
              <OptionalAiRuntimeCard
              embedded
              feature="bandit"
              available={dialogueEnvironment()?.runtimeInstalled}
              modelReady={!!dialogueEnvironment()?.modelCached}
              modelDescription={`model BandIt v2 (khoảng ${formatBytes(dialogueEnvironment()?.estimatedDownloadBytes ?? 446_680_129)})`}
              installDisabledReason={dialogueEnvironmentLoading() ? 'Đang kiểm tra model tách thoại đã cài…' : undefined}
              installModels={async (reportProgress) => { reportProgress('Đang tải model BandIt v2…'); await installDialogueModel(true); }}
              onInstalled={() => { void Promise.all([refreshDialogueEnvironment(), refreshStorage()]); }}
              />
            </div>
            <div class="settings-info-note"><Cpu size={15} /><span>faster-whisper, đọc chữ và VieNeu trên CPU đã đi kèm ứng dụng. Chỉ cài thêm tính năng bạn sử dụng.</span></div>
          </Show>

          <Show when={section() === 'Voice / TTS'}>
            <SettingsSectionHeader title="Giọng đọc" subtitle="VieNeu-TTS chạy trên thiết bị. Mô hình chỉ được tải sau khi bạn xác nhận." />
            <Show when={settings()} fallback={<div class="settings-card provider-settings-card">Đang đọc cài đặt giọng đọc…</div>}>
              <div class="settings-card tts-settings-card">
                <div class="tts-model-status-row">
                  <div class="provider-card-icon"><Volume2 size={17} /></div>
                  <div class="provider-card-main"><span class="provider-eyebrow">GIỌNG VIỆT · TRÊN THIẾT BỊ</span><h3>VieNeu-TTS v3 Turbo</h3>
                    <p>{ttsEnvironmentLoading() ? 'Đang kiểm tra giọng đọc và model…' : !ttsEnvironment()?.workerInstalled ? 'Bộ xử lý giọng đọc chưa sẵn sàng.' : ttsEnvironment()?.modelCached ? 'Model đã có trên thiết bị.' : 'Model chưa được tải về.'}</p>
                  </div>
                  <span class="provider-status-dot" classList={{ 'provider-status-dot-ready': !!ttsEnvironment()?.modelCached }} />
                </div>
                <Show when={!ttsEnvironmentLoading() && !ttsEnvironment()?.workerInstalled}>
                  <div class="tts-install-instructions"><strong>Giọng đọc chưa sẵn sàng</strong><span>Hãy cập nhật JC AI. Model giọng đọc đã tải được giữ nguyên.</span></div>
                </Show>
                <div class="provider-config-grid">
                  <label class="provider-field"><span>Dịch vụ giọng đọc</span><select class="settings-select" value={settings()!.tts.provider} disabled><option value="vieneu">VieNeu-TTS · chạy trên thiết bị</option></select></label>
                  <label class="provider-field"><span>Model</span><select class="settings-select" value={settings()!.tts.modelId} disabled><option value="v3-turbo">VieNeu-TTS v3 Turbo · 48 kHz</option></select></label>
                  <label class="provider-field"><span>Thiết bị</span><select class="settings-select" value={settings()!.tts.device} onChange={(event) => { chooseDevice('tts', event.currentTarget.value as 'cpu' | 'cuda'); if (cudaInstallTarget()) event.currentTarget.value = settings()!.tts.device; }}><option value="cpu">CPU</option><option value="cuda">NVIDIA · CUDA{ttsEnvironment()?.cudaAvailable ? '' : ' · cài khi chọn'}</option></select></label>
                  <label class="provider-field"><span>Giọng mặc định</span><select class="settings-select" value={settings()!.tts.defaultVoice} onChange={(event) => updateTts({ defaultVoice: event.currentTarget.value })}><Show when={!ttsEnvironment()?.voices.includes(settings()!.tts.defaultVoice)}><option value={settings()!.tts.defaultVoice}>{settings()!.tts.defaultVoice}</option></Show><For each={ttsEnvironment()?.voices ?? []}>{(voice) => <option value={voice}>{voice}</option>}</For><For each={settings()!.tts.voiceProfiles}>{(profile) => <option value={profile.name}>{profile.name} · giọng tham chiếu</option>}</For></select></label>
                  <label class="provider-field"><span>Tốc độ <small>{settings()!.tts.speed.toFixed(2)}×</small></span><input type="range" min="0.85" max="2" step="0.01" value={settings()!.tts.speed} onInput={(event) => updateTts({ speed: Number(event.currentTarget.value) })} /></label>
                </div>
                <label class="property-control-row tts-fit-row"><span>Khớp thời lượng câu thoại <small>Giới hạn tăng tốc tối đa 2.00×</small></span><input type="checkbox" checked={settings()!.tts.fitToDuration} onChange={(event) => updateTts({ fitToDuration: event.currentTarget.checked })} /></label>
                <div class="tts-profiles-section"><div class="tts-profiles-heading"><strong>Giọng tùy chỉnh</strong><button class="button button-quiet" type="button" onClick={() => setShowVoiceProfileDialog(true)}>Thêm giọng tham chiếu</button></div>
                  <For each={settings()!.tts.voiceProfiles}>{(profile) => <div class="tts-profile-row"><span><strong>{profile.name}</strong><small>Audio tham chiếu đã lưu cục bộ</small></span><button class="mini-tool-button" type="button" disabled={voiceProfileBusy()} onClick={() => void removeVoiceProfile(profile.id)}>Xóa</button></div>}</For>
                  <Show when={!settings()!.tts.voiceProfiles.length}><small class="tts-profile-empty">Có thể dùng preset VieNeu hoặc thêm audio tham chiếu 3–8 giây. Cần quyền sử dụng và sự đồng ý phù hợp.</small></Show>
                </div>
                <Show when={ttsEnvironment()?.workerInstalled && !ttsEnvironment()?.modelCached}>
                  <button class="button button-primary tts-install-button" type="button" disabled={installingTtsModel()} onClick={() => setShowTtsModelConfirm(true)}>{installingTtsModel() ? 'Đang tải model…' : 'Tải model VieNeu-TTS'}</button>
                </Show>
                <Show when={ttsEnvironment()?.modelCached}><button class="button button-quiet tts-install-button" type="button" disabled={installingTtsModel() || removingTtsModel()} onClick={() => setShowRemoveTtsModelConfirm(true)}>{removingTtsModel() ? 'Đang gỡ model…' : 'Gỡ model VieNeu-TTS'}</button></Show>
                <Show when={installingTtsModel()}><div class="tts-install-progress" role="status"><div><span>{ttsModelMessage()}</span><span>{Math.round(ttsModelProgress() * 100)}%</span></div><div class="job-progress-track"><span style={{ width: `${ttsModelProgress() * 100}%` }} /></div></div></Show>
                <Show when={ttsEnvironmentError()}><div class="provider-feedback provider-feedback-error" role="alert">{ttsEnvironmentError()}</div></Show>
                <div class="provider-action-row">
                  <button class="button button-quiet provider-action-secondary" type="button" disabled={ttsEnvironmentLoading()} onClick={() => void refreshTtsEnvironment()}>{ttsEnvironmentLoading() ? 'Đang kiểm tra…' : 'Kiểm tra lại'}</button>
                  <button class="button button-quiet provider-action-secondary" type="button" disabled={previewBusy() || !ttsEnvironment()?.modelCached} onClick={() => void previewTtsVoice()}>{previewBusy() ? 'Đang tạo giọng thử…' : 'Nghe thử giọng'}</button>
                  <button class="button button-primary" type="button" disabled={busy()} onClick={() => void saveSettings()}>{busy() ? 'Đang lưu…' : 'Lưu cấu hình TTS'}</button>
                </div>
                <Show when={voicePreviewUrl()}><audio class="tts-preview-audio" controls src={voicePreviewUrl()} aria-label="Nghe thử giọng VieNeu" /></Show>
                <Show when={settingsMessage()}><div class="provider-feedback" classList={{ 'provider-feedback-error': settingsError() }} role={settingsError() ? 'alert' : 'status'}>{settingsMessage()}</div></Show>
                <Show when={ttsEnvironment()?.workerInstalled && ttsEnvironment()?.modelCached && settings()!.tts.device === 'cuda' && !ttsEnvironment()?.cudaAvailable}><p class="asr-config-warning" role="alert">Tăng tốc NVIDIA chưa sẵn sàng. Chọn lại NVIDIA trong mục Thiết bị để cài, hoặc dùng CPU.</p></Show>
              </div>
            </Show>
          </Show>

          <Show when={section() === 'Advanced'}>
            <SettingsSectionHeader title="Nâng cao" subtitle="Kiểm tra thành phần ứng dụng và dung lượng lưu trữ." />
            <div class="runtime-health-card"><div class="runtime-health-header"><div><span class="provider-eyebrow">TRẠNG THÁI ỨNG DỤNG</span><h3>Thành phần ứng dụng</h3></div><button class="button button-quiet" type="button" onClick={() => void refreshRuntimeHealth()}>Kiểm tra lại</button></div>
              <div class="runtime-health-grid"><RuntimeStatus name="Lưu trữ dự án" ready={!!runtimeHealth()?.projectStorageAvailable} loading={!runtimeHealth()} /><RuntimeStatus name="Xử lý video" ready={!!runtimeHealth()?.ffmpegAvailable} detail={runtimeHealth()?.ffmpegVersion ?? undefined} loading={!runtimeHealth()} /><RuntimeStatus name="Đọc thông tin video" ready={!!runtimeHealth()?.ffprobeAvailable} loading={!runtimeHealth()} /><RuntimeStatus name="Xử lý AI" ready={!!runtimeHealth()?.workerAvailable} loading={!runtimeHealth()} /></div>
              <Show when={runtimeHealthError()}><div class="provider-feedback provider-feedback-error" role="alert">{runtimeHealthError()}</div></Show>
              <div class="runtime-diagnostics-row"><p>Báo cáo hỗ trợ chỉ gồm trạng thái ứng dụng và tối đa 5 bản ghi lỗi gần nhất. Không gồm video, bản chép lời hoặc thông tin đăng nhập.</p><button class="button button-primary" type="button" disabled={diagnosticsBusy()} onClick={() => void exportDiagnostics()}>{diagnosticsBusy() ? 'Đang tạo…' : 'Xuất báo cáo hỗ trợ'}</button></div>
            </div>
            <div class="settings-card">
              <SettingRow icon={<Cpu size={17} />} title="Giới hạn xử lý" description="Ứng dụng tự sắp xếp các tác vụ nặng để máy hoạt động ổn định."><span class="resource-default-badge">Tự động</span></SettingRow>
              <div class="settings-row-divider" />
              <SettingRow icon={<SlidersHorizontal size={17} />} title="Dữ liệu tạm" description={storageUsage()?.cacheDirectory ?? 'Thư mục dữ liệu tạm của ứng dụng.'}>
                <button class="button button-quiet storage-open-button" type="button" onClick={() => void openStorageFolder(() => storageService.openCacheFolder())}>Mở thư mục</button>
              </SettingRow>
            </div>
            <div class="storage-overview-card">
              <div class="storage-overview-header"><div><span class="provider-eyebrow">LOCAL STORAGE</span><h3>Dung lượng sử dụng</h3></div><button class="button button-quiet" type="button" disabled={storageBusy()} onClick={() => void refreshStorage()}>{storageBusy() ? 'Đang đọc…' : 'Làm mới'}</button></div>
              <div class="storage-stat-grid">
                <div class="storage-model-stat">
                  <small>Mô hình đã tải · tổng</small><strong>{storageUsage() ? formatBytes(storageUsage()!.modelsBytes) : '—'}</strong>
                  <div class="storage-model-breakdown">
                    <div class="storage-model-location"><span><b>Whisper · Intel</b><small title={storageUsage()?.whisperIntelDirectory}>{storageUsage()?.whisperIntelDirectory ?? 'Model Whisper cho GPU Intel'}</small></span><strong>{storageUsage() ? formatBytes(storageUsage()!.whisperIntelBytes) : '—'}</strong><button class="storage-model-open" type="button" aria-label="Mở thư mục Whisper Intel" onClick={() => void openStorageFolder(() => storageService.openModelFolder('whisperIntel'))}><FolderOpen size={13} /></button></div>
                    <div class="storage-model-location"><span><b>faster-whisper</b><small title={storageUsage()?.fasterWhisperDirectory}>{storageUsage()?.fasterWhisperDirectory ?? 'Đường dẫn model Speech Recognition'}</small></span><strong>{storageUsage() ? formatBytes(storageUsage()!.fasterWhisperBytes) : '—'}</strong><button class="storage-model-open" type="button" aria-label="Mở thư mục faster-whisper" title="Mở thư mục faster-whisper" onClick={() => void openStorageFolder(() => storageService.openModelFolder('fasterWhisper'))}><FolderOpen size={13} /></button></div>
                    <div class="storage-model-location"><span><b>RapidOCR</b><small title={storageUsage()?.ocrDirectory}>{storageUsage()?.ocrDirectory ?? 'Đường dẫn model OCR'}</small></span><strong>{storageUsage() ? formatBytes(storageUsage()!.ocrBytes) : '—'}</strong><button class="storage-model-open" type="button" aria-label="Mở thư mục RapidOCR" title="Mở thư mục RapidOCR" onClick={() => void openStorageFolder(() => storageService.openModelFolder('ocr'))}><FolderOpen size={13} /></button></div>
                    <div class="storage-model-location"><span><b>VieNeu-TTS</b><small title={storageUsage()?.vieNeuTtsDirectory}>{storageUsage()?.vieNeuTtsDirectory ?? 'Đường dẫn model Voice / TTS'}</small></span><strong>{storageUsage() ? formatBytes(storageUsage()!.vieNeuTtsBytes) : '—'}</strong><button class="storage-model-open" type="button" aria-label="Mở thư mục VieNeu-TTS" title="Mở thư mục VieNeu-TTS" onClick={() => void openStorageFolder(() => storageService.openModelFolder('vieNeuTts'))}><FolderOpen size={13} /></button></div>
                    <div class="storage-model-location"><span><b>BandIt v2 · giảm thoại</b><small title={storageUsage()?.dialogueSeparationDirectory}>{storageUsage()?.dialogueSeparationDirectory ?? 'Đường dẫn model tách thoại'}</small></span><strong>{storageUsage() ? formatBytes(storageUsage()!.dialogueSeparationBytes) : '—'}</strong><button class="storage-model-open" type="button" aria-label="Mở thư mục BandIt" title="Mở thư mục BandIt" onClick={() => void openStorageFolder(() => storageService.openModelFolder('dialogueSeparation'))}><FolderOpen size={13} /></button></div>
                    <Show when={(storageUsage()?.otherModelDataBytes ?? 0) > 0}><span class="storage-model-other">Khác · profile giọng hoặc model cũ: {formatBytes(storageUsage()!.otherModelDataBytes)}</span></Show>
                  </div>
                </div>
                <div><small>Dự án</small><strong>{storageUsage() ? formatBytes(storageUsage()!.projectsBytes) : '—'}</strong><span>Video, bản chép lời, âm thanh và video đã xuất</span></div>
                <div><small>Xử lý AI bổ sung</small><strong>{storageUsage() ? formatBytes(storageUsage()!.aiComponentsBytes) : '—'}</strong><span title={storageUsage()?.aiComponentsDirectory}>Bộ xử lý CPU hoặc NVIDIA đã cài</span><button class="storage-model-open" type="button" aria-label="Mở thư mục xử lý AI bổ sung" title="Mở thư mục xử lý AI bổ sung" onClick={() => void openStorageFolder(() => storageService.openAiComponentsFolder())}><FolderOpen size={13} /></button></div>
                <div><small>Cache cài đặt AI</small><strong>{storageUsage() ? formatBytes(storageUsage()!.aiInstallationCacheBytes) : '—'}</strong><span>Gói tải dùng lại cho lần cài sau, giúp tiết kiệm mạng</span><button class="button button-quiet" type="button" disabled={storageBusy() || !storageUsage()?.aiInstallationCacheBytes} onClick={() => setShowClearInstallCacheConfirm(true)}>Xóa cache cài đặt</button></div>
                <div><small>Dữ liệu xem trước</small><strong>{storageUsage() ? formatBytes(storageUsage()!.projectCacheBytes) : '—'}</strong><span>Dữ liệu dùng để mở dự án nhanh hơn</span></div>
                <div><small>Tệp tạm</small><strong>{storageUsage() ? formatBytes(storageUsage()!.temporaryBytes) : '—'}</strong><span>Tệp xuất video và lượt tải chưa xong</span></div>
                <div><small>Bản ghi ứng dụng</small><strong>{storageUsage() ? formatBytes(storageUsage()!.logsBytes) : '—'}</strong><span>Thông tin hỗ trợ khi có lỗi</span></div>
              </div>
              <Show when={(storageUsage()?.warnings.length ?? 0) > 0}><div class="provider-feedback" role="status"><For each={storageUsage()?.warnings}>{(warning) => <div>{warning}</div>}</For></div></Show>
              <div class="storage-cleanup-row"><p>Chỉ xóa tệp tạm và lượt tải chưa xong. Video, bản chép lời, bản dịch, âm thanh, video đã xuất và mô hình được giữ nguyên.</p><button class="button button-quiet" type="button" disabled={storageBusy() || !storageUsage()?.temporaryBytes} onClick={() => { setStorageBusy(true); void storageService.clearTemporary().then((bytes) => { showMessage(`Đã dọn ${formatBytes(bytes)} tệp tạm.`, false); return refreshStorage(); }).catch((error) => setStorageError(error instanceof Error ? error.message : String(error))).finally(() => setStorageBusy(false)); }}>Dọn tệp tạm</button></div>
              <Show when={storageError()}><div class="provider-feedback provider-feedback-error" role="alert">{storageError()}</div></Show>
            </div>
              <div class="settings-info-note"><CircleHelp size={15} /><span>Ứng dụng giới hạn số tác vụ nặng chạy cùng lúc để máy hoạt động ổn định.</span></div>
          </Show>

          <Show when={section() === 'About'}>
            <SettingsSectionHeader title="Giới thiệu" subtitle="Phiên bản và giấy phép các thành phần." />
            <div class="settings-card about-card">
              <SettingRow icon={<Sparkles size={17} />} title="JC AI" description="Ứng dụng dịch và lồng tiếng video trên Windows."><span class="version-chip">{runtimeHealth()?.appVersion ?? '…'}</span></SettingRow>
              <div class="settings-row-divider" />
              <SettingRow icon={<SlidersHorizontal size={17} />} title="Phiên bản phát hành" description="Thông tin phiên bản để hỗ trợ khi cần."><span class="version-chip">{runtimeHealth()?.buildCommit ?? '—'} · {runtimeHealth()?.os ?? '—'} {runtimeHealth()?.architecture ?? ''}</span></SettingRow>
              <div class="settings-row-divider" />
              <SettingRow icon={<Cpu size={17} />} title="Xử lý video" description="Dùng để đọc, xem trước và xuất video."><span class="version-chip">{runtimeHealth()?.ffmpegVersion ?? 'Không khả dụng'}</span></SettingRow>
              <div class="settings-row-divider" />
              <SettingRow icon={<AudioLines size={17} />} title="Xử lý AI" description="Được cài cùng ứng dụng; không cần cài đặt riêng để dùng tính năng cơ bản."><span class="version-chip">{runtimeHealth()?.workerVersion ?? '—'}</span></SettingRow>
            </div>
            <div class="settings-info-note"><Info size={15} /><span>Giấy phép của các thành phần đi kèm nằm trong thư mục cài đặt. Mỗi mô hình tải thêm có điều khoản riêng từ nhà cung cấp.</span></div>
          </Show>
        </section>
      </div>
      <Show when={cudaInstallTarget()}>{(target) =>
        <Modal title={`Dùng NVIDIA cho ${target() === 'tts' ? 'giọng đọc' : target() === 'ocr' ? 'đọc chữ' : 'nhận dạng lời thoại'}?`}
          description="Ứng dụng sẽ cài thành phần CUDA và model đang chọn trong cùng một lượt. Thành phần đã có được dùng lại. Cần Internet và đủ dung lượng trống."
          onClose={() => { if (!cudaInstalling()) setCudaInstallTarget(undefined); }}>
          <div class="model-confirm-card"><div class="job-banner-icon"><Sparkles size={17} /></div><div><strong>{target() === 'tts' ? 'VieNeu-TTS v3 Turbo' : target() === 'ocr' ? `RapidOCR · ${settings()?.asr.ocrModel}` : `${settings()?.asr.engine === 'whisperX' ? 'WhisperX' : 'faster-whisper'} · ${settings()?.asr.model}`}</strong><p>Phần CUDA dùng chung có thể cần khoảng 6–7 GB khi cài lần đầu. Model đã tải sẽ không bị tải lại.</p></div></div>
          <Show when={cudaInstalling()}><p class="provider-help" role="status"><span class="spin status-spinner" /> {cudaInstallProgress()}</p></Show>
          <Show when={cudaInstallError()}><div class="provider-feedback provider-feedback-error" role="alert">{cudaInstallError()}</div></Show>
          <footer class="modal-actions"><button class="button button-quiet" type="button" disabled={cudaInstalling()} onClick={() => setCudaInstallTarget(undefined)}>Hủy</button><button class="button button-primary" type="button" disabled={cudaInstalling()} onClick={() => void installCudaFor()}>{cudaInstalling() ? 'Đang cài…' : 'Tải và dùng CUDA'}</button></footer>
        </Modal>
      }</Show>
      <Show when={showAsrModelConfirm()}>
        <Modal title={`Cài model faster-whisper ${settings()?.asr.model ?? ''}?`} description="Model được cài bền vững trong thư mục faster-whisper dùng chung cho các project. Cài đặt bắt đầu sau khi bạn xác nhận." onClose={() => setShowAsrModelConfirm(false)}>
          <div class="model-confirm-card"><div class="job-banner-icon"><Cpu size={17} /></div><div><strong>{settings()?.asr.model} · khoảng {estimatedModelSize(settings()?.asr.model ?? 'small')}</strong><p>Model được tải một lần từ Hugging Face và giữ cục bộ. Cần kết nối Internet và đủ dung lượng trống. Tác vụ Generate Transcript không tải model.</p></div></div>
          <footer class="modal-actions"><button class="button button-quiet" type="button" onClick={() => setShowAsrModelConfirm(false)}>Hủy</button><button class="button button-primary" type="button" onClick={() => void installAsrModel()}>Tải model</button></footer>
        </Modal>
      </Show>
      <Show when={showRemoveAsrModelConfirm()}>
        <Modal title={`Gỡ model faster-whisper ${settings()?.asr.model ?? ''}?`} description="Chỉ xóa model faster-whisper đã cài. Transcript và dữ liệu dự án không bị xóa." onClose={() => setShowRemoveAsrModelConfirm(false)}>
          <footer class="modal-actions"><button class="button button-quiet" type="button" onClick={() => setShowRemoveAsrModelConfirm(false)}>Hủy</button><button class="button button-primary" type="button" onClick={() => void removeAsrModel()}>Gỡ model</button></footer>
        </Modal>
      </Show>
      <Show when={showOcrModelConfirm()}>
        <Modal title={`Cài ${ocrEnvironment()?.model ?? 'PP-OCRv6 Tiny'}?`} description="Trọng số OCR được tải riêng vào thư mục Models dùng chung. Tải chỉ bắt đầu sau khi bạn xác nhận." onClose={() => setShowOcrModelConfirm(false)}>
          <div class="model-confirm-card"><div class="job-banner-icon"><Sparkles size={17} /></div><div><strong>{ocrEnvironment()?.model ?? 'PP-OCRv6 Tiny'} · khoảng {formatBytes(ocrEnvironment()?.estimatedDownloadBytes ?? 7 * 1024 * 1024)}</strong><p>{settings()?.asr.ocrModel === 'tiny' ? 'Tiny ưu tiên tốc độ và dung lượng nhỏ.' : settings()?.asr.ocrModel === 'small' ? 'Small cân bằng tốc độ và độ chính xác, dung lượng lớn hơn Tiny.' : 'Medium là model lớn, dùng nhiều bộ nhớ và phù hợp khi ưu tiên độ chính xác.'} Lần đầu cần Internet; sau khi cài, transcript chạy offline. Tiến trình sẽ hiển thị dung lượng tải.</p></div></div>
          <footer class="modal-actions"><button class="button button-quiet" type="button" onClick={() => setShowOcrModelConfirm(false)}>Hủy</button><button class="button button-primary" type="button" onClick={() => void installOcrModel()}>Tải model OCR</button></footer>
        </Modal>
      </Show>
      <Show when={showSttnModelConfirm()}>
        <Modal title="Cài model STTN?" description="Model STTN được tải riêng với bộ xử lý CPU/NVIDIA và lưu dùng chung trên thiết bị. RapidOCR chỉ cần khi chạy chức năng xóa chữ." onClose={() => setShowSttnModelConfirm(false)}>
          <div class="model-confirm-card"><div class="job-banner-icon"><Sparkles size={17} /></div><div><strong>STTN · khoảng {formatBytes(textRemovalEnvironment()?.sttnEstimatedDownloadBytes ?? 66_252_587)}</strong><p>Model tải một lần và dùng được cho mọi project.</p></div></div>
          <footer class="modal-actions"><button class="button button-quiet" type="button" onClick={() => setShowSttnModelConfirm(false)}>Hủy</button><button class="button button-primary" type="button" disabled={installingSttnModel()} onClick={() => void installSttnModel()}>{installingSttnModel() ? 'Đang cài…' : 'Tải và cài model'}</button></footer>
        </Modal>
      </Show>
      <Show when={showRemoveSttnModelConfirm()}>
        <Modal title="Gỡ model STTN?" description="Chỉ xóa trọng số model STTN. Bộ xử lý STTN, project và các model khác được giữ nguyên." onClose={() => setShowRemoveSttnModelConfirm(false)}>
          <div class="model-confirm-card"><div class="job-banner-icon"><Trash2 size={17} /></div><div><strong>{formatBytes(textRemovalEnvironment()?.sttnModelBytes ?? 0)} sẽ được giải phóng</strong><p>Bạn có thể tải lại model trong Cài đặt ứng dụng.</p></div></div>
          <footer class="modal-actions"><button class="button button-quiet" type="button" onClick={() => setShowRemoveSttnModelConfirm(false)}>Giữ model</button><button class="button button-danger" type="button" disabled={removingSttnModel()} onClick={() => void removeSttnModel()}>{removingSttnModel() ? 'Đang gỡ…' : 'Gỡ model và file'}</button></footer>
        </Modal>
      </Show>
      <Show when={showRemoveOcrModelConfirm()}>
        <Modal title={`Gỡ ${ocrEnvironment()?.model ?? 'mô hình đọc chữ'}?`} description="Chỉ xóa mô hình đang chọn. Các mô hình khác và dữ liệu dự án được giữ nguyên." onClose={() => setShowRemoveOcrModelConfirm(false)}>
          <div class="model-confirm-card"><div class="job-banner-icon"><Sparkles size={17} /></div><div><strong>{formatBytes(ocrEnvironment()?.modelBytes ?? 0)} sẽ được giải phóng</strong><p>Dữ liệu của {ocrEnvironment()?.model ?? 'mô hình đang chọn'} sẽ bị xóa. Bạn có thể tải lại trong Cài đặt → Nhận dạng lời thoại.</p></div></div>
          <footer class="modal-actions"><button class="button button-quiet" type="button" onClick={() => setShowRemoveOcrModelConfirm(false)}>Giữ model</button><button class="button button-danger" type="button" disabled={removingOcrModel()} onClick={() => void removeOcrModel()}>{removingOcrModel() ? 'Đang xóa…' : 'Xóa model và file'}</button></footer>
        </Modal>
      </Show>
      <Show when={showRemoveDialogueModelConfirm()}>
        <Modal title="Xóa mô hình BandIt v2?" description="Mô hình giảm thoại sẽ được xóa. Âm thanh và video đã tạo trong các dự án được giữ nguyên." onClose={() => setShowRemoveDialogueModelConfirm(false)}>
          <div class="model-confirm-card"><div class="job-banner-icon"><AudioLines size={17} /></div><div><strong>{formatBytes(dialogueEnvironment()?.modelBytes ?? 0)} sẽ được giải phóng</strong><p>Đang export thì không thể gỡ model. Có thể tải lại model sau.</p></div></div>
          <footer class="modal-actions"><button class="button button-quiet" type="button" onClick={() => setShowRemoveDialogueModelConfirm(false)}>Giữ model</button><button class="button button-danger" type="button" disabled={removingDialogueModel()} onClick={() => void removeDialogueModel()}>{removingDialogueModel() ? 'Đang xóa…' : 'Xóa model và file'}</button></footer>
        </Modal>
      </Show>
      <Show when={showClearInstallCacheConfirm()}>
        <Modal title="Xóa cache cài đặt AI?" description="Các tính năng AI và model đã cài vẫn dùng bình thường. Khi cài thêm tính năng, ứng dụng có thể cần tải lại các gói đã xóa." onClose={() => setShowClearInstallCacheConfirm(false)}>
          <p class="provider-help">Có thể giải phóng {formatBytes(storageUsage()?.aiInstallationCacheBytes ?? 0)}. Giữ lại cache nếu bạn muốn tiết kiệm dung lượng mạng cho lần cài sau.</p>
          <footer class="modal-actions"><button class="button button-quiet" type="button" onClick={() => setShowClearInstallCacheConfirm(false)}>Giữ lại</button><button class="button button-danger" type="button" disabled={storageBusy()} onClick={() => void clearInstallCache()}>Xóa cache cài đặt</button></footer>
        </Modal>
      </Show>
      <Show when={showTtsModelConfirm()}>
        <Modal title="Tải model VieNeu-TTS?" description="Model và bộ giải mã được lưu bền vững trong thư mục VieNeu-TTS dùng chung cho các project. Tải chỉ bắt đầu sau khi bạn xác nhận." onClose={() => setShowTtsModelConfirm(false)}>
          <div class="model-confirm-card"><div class="job-banner-icon"><Cpu size={17} /></div><div><strong>VieNeu-TTS v3 Turbo · khoảng 1.8 GB</strong><p>Model và bộ giải mã tải từ Hugging Face. Dung lượng thay đổi theo phiên bản và file đi kèm. Cần kết nối Internet và đủ dung lượng trống.</p></div></div>
          <footer class="modal-actions"><button class="button button-quiet" type="button" onClick={() => setShowTtsModelConfirm(false)}>Hủy</button><button class="button button-primary" type="button" onClick={() => void installTtsModel()}>Tải model</button></footer>
        </Modal>
      </Show>
      <Show when={showRemoveTtsModelConfirm()}>
        <Modal title="Gỡ mô hình VieNeu-TTS?" description="Chỉ xóa mô hình giọng đọc đã tải. Dự án, bản chép lời và âm thanh đã tạo được giữ nguyên." onClose={() => setShowRemoveTtsModelConfirm(false)}>
          <footer class="modal-actions"><button class="button button-quiet" type="button" onClick={() => setShowRemoveTtsModelConfirm(false)}>Hủy</button><button class="button button-primary" type="button" onClick={() => void removeTtsModel()}>Gỡ model</button></footer>
        </Modal>
      </Show>
      <Show when={showVoiceProfileDialog()}>
        <Modal title="Thêm giọng tùy chỉnh" description="Chọn đoạn ghi âm mẫu dài 3–8 giây. Ứng dụng lưu bản sao trên thiết bị; tệp gốc không bị thay đổi." onClose={() => setShowVoiceProfileDialog(false)}>
          <div class="voice-profile-form">
            <label class="provider-field"><span>Tên giọng</span><input value={voiceProfileName()} maxLength={48} onInput={(event) => setVoiceProfileName(event.currentTarget.value)} placeholder="Ví dụ: Giọng dẫn chuyện" /></label>
            <div class="voice-profile-file-row"><span>{voiceProfileFile() ? voiceProfileFile().split(/[\\/]/).pop() : 'Chưa chọn audio tham chiếu'}</span><button class="button button-quiet" type="button" onClick={() => void browseVoiceReference()}>Chọn audio</button></div>
            <label class="voice-profile-consent"><input type="checkbox" checked={voiceProfileConsent()} onChange={(event) => setVoiceProfileConsent(event.currentTarget.checked)} /><span>Tôi có quyền sử dụng audio này và đã có sự đồng ý cần thiết từ người có giọng nói.</span></label>
            <p class="provider-help">Ứng dụng sẽ kiểm tra thời lượng và lưu bản ghi âm mẫu để dùng cho các dự án.</p>
          </div>
          <footer class="modal-actions"><button class="button button-quiet" type="button" onClick={() => setShowVoiceProfileDialog(false)}>Hủy</button><button class="button button-primary" type="button" disabled={!voiceProfileName().trim() || !voiceProfileFile() || !voiceProfileConsent() || voiceProfileBusy()} onClick={() => void addVoiceProfile()}>{voiceProfileBusy() ? 'Đang kiểm tra âm thanh…' : 'Thêm giọng'}</button></footer>
        </Modal>
      </Show>
    </main>
  );
}

function SettingsSectionHeader(props: { title: string; subtitle: string }) {
  return <header class="settings-detail-header"><div class="settings-detail-kicker">CÀI ĐẶT ỨNG DỤNG <span>/</span> {props.title.toLocaleUpperCase()}</div><h2>{props.title}</h2><p>{props.subtitle}</p></header>;
}

function SettingRow(props: { icon: import('solid-js').JSX.Element; title: string; description: string; children: import('solid-js').JSX.Element }) {
  return <div class="settings-row"><span class="settings-row-icon">{props.icon}</span><div class="settings-row-copy"><strong>{props.title}</strong><span>{props.description}</span></div><div class="settings-row-value">{props.children}</div></div>;
}

function RuntimeStatus(props: { name: string; ready: boolean; detail?: string; loading?: boolean }) {
  return <div class="runtime-status"><span class="runtime-status-dot" classList={{ ready: props.ready, loading: props.loading }} /><div><strong>{props.name}</strong><small>{props.loading ? 'Đang kiểm tra…' : props.ready ? props.detail || 'Sẵn sàng' : 'Không khả dụng'}</small></div></div>;
}
