import { createSignal, For, onMount, Show } from 'solid-js';
import { currentProject, setCurrentProject, setNotice } from '../../app/project-state';
import { projectService } from '../../app/services/project-service';
import { translationService } from '../../app/services/translation-service';
import { ttsService } from '../../app/services/tts-service';
import type { Project } from '../projects/types';
import type { TtsEnvironment } from '../transcript/types';
import Modal from '../../shared/components/Modal';

export default function ProjectSettingsDialog(props: { project: Project; onClose: () => void }) {
  const [customPrompt, setCustomPrompt] = createSignal(props.project.translation.customPrompt ?? '');
  const [translationContext, setTranslationContext] = createSignal(props.project.translation.context ?? '');
  const [transcriptMode, setTranscriptMode] = createSignal<'speechRecognition' | 'ocr'>(props.project.transcriptSettings?.mode ?? 'speechRecognition');
  const [ocrFps, setOcrFps] = createSignal<1 | 2 | 4>(props.project.transcriptSettings?.ocrFps ?? 2);
  const [voiceValue, setVoiceValue] = createSignal('');
  const [voiceProfiles, setVoiceProfiles] = createSignal<Array<{ id: string; name: string; audioPath: string }>>([]);
  const [environment, setEnvironment] = createSignal<TtsEnvironment>();
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal('');
  const [saving, setSaving] = createSignal(false);

  const voiceName = () => {
    const profile = voiceProfiles().find((item) => item.id === props.project.dubbing.voiceProfileId);
    return profile?.name ?? props.project.dubbing.voice ?? '';
  };

  onMount(() => {
    void Promise.allSettled([translationService.getSettings(), ttsService.getEnvironment()])
      .then(([settingsResult, environmentResult]) => {
        if (environmentResult.status === 'fulfilled') setEnvironment(environmentResult.value);
        if (settingsResult.status === 'fulfilled') {
          const appSettings = settingsResult.value.settings.tts;
          setVoiceProfiles(appSettings.voiceProfiles);
          const selectedVoice = props.project.dubbing.voice?.trim() || appSettings.defaultVoice;
          const profile = props.project.dubbing.voiceProfileId
            ? appSettings.voiceProfiles.find((item) => item.id === props.project.dubbing.voiceProfileId)
            : appSettings.voiceProfiles.find((item) => item.name === selectedVoice);
          setVoiceValue(profile ? `profile:${profile.id}` : `preset:${selectedVoice}`);
        } else {
          const savedProfileId = props.project.dubbing.voiceProfileId;
          const savedVoice = props.project.dubbing.voice?.trim();
          if (savedProfileId) setVoiceValue(`profile:${savedProfileId}`);
          else if (savedVoice) setVoiceValue(`preset:${savedVoice}`);
          setError(settingsResult.reason instanceof Error ? settingsResult.reason.message : 'Không thể đọc cài đặt TTS.');
        }
        if (environmentResult.status === 'rejected' && settingsResult.status === 'fulfilled') {
          setError(environmentResult.reason instanceof Error ? environmentResult.reason.message : 'Không thể kiểm tra giọng TTS.');
        }
      })
      .finally(() => setLoading(false));
  });

  const save = async (event: SubmitEvent) => {
    event.preventDefault();
    if (saving()) return;
    const selected = voiceValue();
    let voice: string | null = null;
    let voiceProfileId: string | null = null;
    if (selected.startsWith('profile:')) {
      voiceProfileId = selected.slice('profile:'.length);
      voice = voiceProfiles().find((item) => item.id === voiceProfileId)?.name ?? '';
    } else if (selected.startsWith('preset:')) {
      voice = selected.slice('preset:'.length);
    }
    if (selected && !voice) {
      setError('Giọng đã chọn không còn khả dụng. Chọn giọng khác trong danh sách.');
      return;
    }

    setSaving(true);
    setError('');
    try {
      const updated = await projectService.updateProjectPreferences(props.project.id, {
        customPrompt: customPrompt(),
        translationContext: translationContext(),
        voice,
        voiceProfileId,
        transcriptSettings: {
          mode: transcriptMode(),
          ocrFps: ocrFps(),
          ocrRegion: props.project.transcriptSettings?.ocrRegion ?? null,
        },
      });
      if (currentProject()?.id === updated.id) setCurrentProject(updated);
      setNotice({ kind: 'success', message: 'Đã lưu prompt, context dịch, giọng TTS và cách tạo transcript cho dự án này.' });
      props.onClose();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Không thể lưu cài đặt dự án.');
    } finally {
      setSaving(false);
    }
  };

  const selectedVoiceUnavailable = () => {
    const value = voiceValue();
    if (value.startsWith('profile:')) return !voiceProfiles().some((profile) => `profile:${profile.id}` === value);
    return !(environment()?.voices ?? []).includes(value.slice('preset:'.length));
  };

  return (
    <Modal
      title={`Cài đặt dự án · ${props.project.name}`}
      description="Các tùy chọn này chỉ áp dụng cho dự án hiện tại. Provider, model và credentials vẫn dùng chung toàn ứng dụng."
      onClose={props.onClose}
    >
      <form class="project-settings-form" onSubmit={(event) => void save(event)}>
        <section class="project-setting-section" aria-labelledby="project-translation-prompt-title">
          <div class="project-setting-heading"><div><span class="provider-eyebrow">TRANSLATION</span><h3 id="project-translation-prompt-title">Prompt dịch</h3></div><span class="project-setting-scope">THEO DỰ ÁN</span></div>
          <div class="translation-prompt-default" role="note">
            <strong>Yêu cầu dịch mặc định · luôn bật</strong>
            <p>Dịch từ ngôn ngữ nguồn sang ngôn ngữ đích, giữ nguyên ID và ranh giới segment. Hướng dẫn dưới đây chỉ bổ sung phong cách, không thể thay thế yêu cầu này.</p>
          </div>
          <label class="provider-field project-custom-prompt">
            <span class="translation-prompt-label-row"><span>Hướng dẫn riêng cho dự án</span><small>{customPrompt().length}/8.000</small></span>
            <textarea maxlength="8000" rows="4" value={customPrompt()} placeholder="Ví dụ: Dùng giọng tự nhiên, thân mật; giữ cách xưng hô nhất quán…" onInput={(event) => setCustomPrompt(event.currentTarget.value)} />
          </label>
          <p class="provider-help">Gemini và Codex CLI dùng hướng dẫn này. Google Cloud Translation NMT không hỗ trợ prompt.</p>
        </section>

        <section class="project-setting-section" aria-labelledby="project-translation-context-title">
          <div class="project-setting-heading"><div><span class="provider-eyebrow">TRANSLATION MEMORY</span><h3 id="project-translation-context-title">Context dịch</h3></div><span class="project-setting-scope">THEO DỰ ÁN</span></div>
          <div class="translation-prompt-default" role="note">
            <strong>Ghi chú dùng xuyên suốt dự án</strong>
            <p>Thêm tên nhân vật, cách dịch tên riêng, thuật ngữ, bối cảnh và quan hệ giữa các nhân vật. Mỗi ý nên nằm trên một dòng.</p>
          </div>
          <label class="provider-field project-custom-prompt">
            <span class="translation-prompt-label-row"><span>Context hiện tại</span><small>{translationContext().length}/20.000</small></span>
            <textarea maxlength="20000" rows="7" value={translationContext()} placeholder={'Nhân vật: Alex Miller → Trương Vĩ\nBối cảnh: câu chuyện lấy bối cảnh trong một môn phái tu tiên\nThuật ngữ: core realm → cảnh giới cốt lõi'} onInput={(event) => setTranslationContext(event.currentTarget.value)} />
          </label>
          <p class="provider-help">Gemini và Codex CLI nhận context này ở mỗi batch dịch. Sau mỗi batch hợp lệ, AI chỉ đề xuất các ý mới; ứng dụng sẽ thêm ý chưa có mà không thay thế ghi chú hiện tại. Google Cloud Translation NMT không hỗ trợ context prompt.</p>
        </section>

        <section class="project-setting-section" aria-labelledby="project-tts-voice-title">
          <div class="project-setting-heading"><div><span class="provider-eyebrow">VOICE · TTS</span><h3 id="project-tts-voice-title">Giọng lồng tiếng</h3></div><span class="project-setting-scope">THEO DỰ ÁN</span></div>
          <label class="provider-field">
            <span>Giọng mặc định của dự án</span>
            <select class="settings-select" value={voiceValue()} disabled={loading()} onChange={(event) => setVoiceValue(event.currentTarget.value)}>
              <Show when={voiceValue() && selectedVoiceUnavailable()}><option value={voiceValue()}>{voiceName() || 'Giọng đã chọn'} · không khả dụng</option></Show>
              <For each={environment()?.voices ?? []}>{(voice) => <option value={`preset:${voice}`}>{voice}</option>}</For>
              <For each={voiceProfiles()}>{(profile) => <option value={`profile:${profile.id}`}>{profile.name} · giọng tham chiếu</option>}</For>
            </select>
          </label>
          <Show when={loading()}><p class="provider-help">Đang đọc danh sách giọng…</p></Show>
          <Show when={!loading() && !environment()?.voices.length && !voiceProfiles().length}><p class="provider-help">Chưa có giọng khả dụng. Cài VieNeu-TTS hoặc thêm voice profile trong Cài đặt ứng dụng → Voice / TTS.</p></Show>
        </section>

        <section class="project-setting-section" aria-labelledby="project-transcript-mode-title">
          <div class="project-setting-heading"><div><span class="provider-eyebrow">TRANSCRIPT</span><h3 id="project-transcript-mode-title">Cách lấy transcript</h3></div><span class="project-setting-scope">THEO DỰ ÁN</span></div>
          <label class="provider-field">
            <span>Phương thức</span>
            <select class="settings-select" value={transcriptMode()} onChange={(event) => setTranscriptMode(event.currentTarget.value as 'speechRecognition' | 'ocr')}>
              <option value="speechRecognition">Lời thoại · faster-whisper / WhisperX</option>
              <option value="ocr">Subtitle đã cháy vào hình · OCR</option>
            </select>
          </label>
          <Show when={transcriptMode() === 'ocr'}>
            <div class="provider-config-grid">
              <label class="provider-field"><span>Lấy mẫu khung hình</span><select class="settings-select" value={ocrFps()} onChange={(event) => setOcrFps(Number(event.currentTarget.value) as 1 | 2 | 4)}><option value="1">1 frame/s · nhanh, ít tài nguyên</option><option value="2">2 frame/s · cân bằng · khuyên dùng</option><option value="4">4 frame/s · bắt câu ngắn tốt hơn</option></select></label>
            </div>
            <p class="provider-help">Chọn vùng subtitle bằng cách kéo trên video ở panel Transcript. OCR chỉ xử lý vùng đã chọn, chạy trên CPU và dùng chung model OCR đã cài trong Cài đặt ứng dụng.</p>
          </Show>
        </section>

        <Show when={error()}><div class="provider-feedback provider-feedback-error" role="alert">{error()}</div></Show>
        <footer class="modal-actions project-settings-actions">
          <button class="button button-quiet" type="button" disabled={saving()} onClick={props.onClose}>Hủy</button>
          <button class="button button-primary" type="submit" disabled={saving() || loading()}>{saving() ? 'Đang lưu…' : 'Lưu cài đặt dự án'}</button>
        </footer>
      </form>
    </Modal>
  );
}
