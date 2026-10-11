import type { PreviewParticipant } from './preview-clock';

/** The browser replaces pending seeks; a token also cancels old frame/play callbacks.
 * No full-file decoding or preview rendering is needed. */
export function mediaPlayer(media: HTMLMediaElement, options: {
  position(timeUs: number): number | null; rate(): number; frameUs(): number;
  gain?(playing: boolean): void; visibility?(ready: boolean): void;
  fail(message: string): void;
}): PreviewParticipant & { dispose(): void; prefetch(timeUs: number): void } {
  let disposed = false, revision = -1, target = -1, token = 0, frameHandle = 0;
  let fallbackTimer: ReturnType<typeof setTimeout> | undefined;
  let pending = false, decoded = false, desired = false, playPending = false, failure: string | undefined;
  const video = media instanceof HTMLVideoElement ? media : undefined;
  const cancelFrame = () => { if (frameHandle) video?.cancelVideoFrameCallback?.(frameHandle); frameHandle = 0; clearTimeout(fallbackTimer); fallbackTimer = undefined; };
  const fail = (message: string) => { if (disposed) return; failure = message; desired = false; media.pause(); options.gain?.(false); options.visibility?.(false); options.fail(message); };
  function completed() {
    if (disposed || media.seeking || target < 0 || Math.abs(media.currentTime - target) > options.frameUs() / 1e6 + .003) return;
    const generation = token; cancelFrame();
    if (video?.requestVideoFrameCallback) {
      frameHandle = video.requestVideoFrameCallback((_now, frame) => {
        frameHandle = 0;
        if (disposed || generation !== token || media.seeking || Math.abs(frame.mediaTime - target) > options.frameUs() / 1e6 + .003) return;
        pending = false; decoded = true; options.visibility?.(true);
      });
      // A paused video can have presented the sought frame BEFORE seeked fires,
      // so no further frame callback arrives. HAVE_CURRENT_DATA + seeked confirms
      // decoding; allow two display ticks for presentation, still checking token.
      fallbackTimer = setTimeout(() => {
        if (disposed || generation !== token || media.seeking || media.readyState < 2 || Math.abs(media.currentTime - target) > options.frameUs() / 1e6 + .003) return;
        cancelFrame(); pending = false; decoded = true; options.visibility?.(true);
      }, 40);
    } else { pending = false; decoded = true; options.visibility?.(true); }
  }
  function prepare(timeUs: number, seekRevision: number, playing: boolean) {
    if (disposed || failure || media.readyState < 2) return false;
    const position = options.position(timeUs); if (position === null) return false;
    const seconds = position / 1e6;
    const tolerance = Math.max(.025, options.frameUs() / 1e6);
    media.playbackRate = Math.max(.0625, Math.min(16, options.rate()));
    const discontinuity = revision !== seekRevision;
    const drift = Math.abs(media.currentTime - seconds);
    const needsSeek = discontinuity || (!pending && drift > (playing ? tolerance + .025 : .000002));
    if (needsSeek) {
      const reusable = decoded && drift < .000002 && !media.seeking;
      revision = seekRevision; target = seconds; token++; cancelFrame();
      media.pause(); options.gain?.(false); options.visibility?.(false);
      pending = true; decoded = false;
      // Setting an unchanged currentTime does not always fire seeked.
      if (reusable) { pending = false; decoded = true; options.visibility?.(true); }
      else { try { media.currentTime = seconds; } catch (e) { fail(String(e)); } }
    }
    if (media.seeking || pending || !decoded) return false;
    return media.readyState >= 2;
  }
  function play(playing: boolean) {
    const next = playing && !disposed && !failure && !pending && !media.seeking;
    if (desired !== next) { desired = next; if (!next) token++; }
    options.gain?.(next);
    if (!next) { media.pause(); return; }
    if (media.paused && !playPending) {
      playPending = true; const generation = token;
      void media.play().then(() => { if (disposed || generation !== token || !desired) media.pause(); })
        .catch(e => { if (!disposed && generation === token && desired && e?.name !== 'AbortError') fail('Nguồn chưa phát được. Kiểm tra file hoặc thử lại.'); })
        .finally(() => { playPending = false; });
    }
  }
  const error = () => { if (media.error) fail('Không giải mã được nguồn trên timeline. Kiểm tra file và thử lại.'); };
  media.addEventListener('seeked', completed); media.addEventListener('loadeddata', completed); media.addEventListener('error', error);
  return {
    prepare, play, error: () => failure,
    prefetch(timeUs) { if (revision < 0) prepare(timeUs, 0, false); },
    dispose() {
      disposed = true; token++; cancelFrame(); media.pause(); options.gain?.(false);
      media.removeEventListener('seeked', completed); media.removeEventListener('loadeddata', completed); media.removeEventListener('error', error);
      media.removeAttribute('src'); media.load();
    },
  };
}
