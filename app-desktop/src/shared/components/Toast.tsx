import { onCleanup, createEffect } from 'solid-js';
import { AlertTriangle, Check, Info, X } from 'lucide-solid';
import { notice, setNotice } from '../../app/project-state';

export default function Toast() {
  let timeout: number | undefined;
  createEffect(() => {
    const current = notice();
    window.clearTimeout(timeout);
    if (current) timeout = window.setTimeout(() => setNotice(), 5400);
  });
  onCleanup(() => window.clearTimeout(timeout));

  return (
    <div class="toast-stack" aria-live="polite" aria-relevant="additions text">
      {notice() && (
        <div class={`toast toast-${notice()!.kind}`} role={notice()!.kind === 'error' ? 'alert' : 'status'}>
          <span class="toast-icon" aria-hidden="true">
            {notice()!.kind === 'success' ? <Check size={17} /> : notice()!.kind === 'error' ? <AlertTriangle size={17} /> : <Info size={17} />}
          </span>
          <span>{notice()!.message}</span>
          <button class="icon-button toast-close" aria-label="Đóng thông báo" onClick={() => setNotice()}><X size={15} /></button>
        </div>
      )}
    </div>
  );
}
