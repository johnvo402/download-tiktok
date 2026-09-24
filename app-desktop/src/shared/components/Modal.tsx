import { onMount, Show } from 'solid-js';
import type { JSX } from 'solid-js';
import { X } from 'lucide-solid';

export default function Modal(props: {
  title: string;
  description?: string;
  onClose: () => void;
  children: JSX.Element;
  danger?: boolean;
}) {
  let panel!: HTMLElement;
  onMount(() => panel.focus());

  return (
    <div class="modal-backdrop" onMouseDown={(event) => event.target === event.currentTarget && props.onClose()}>
      <section
        ref={panel}
        class="modal-panel"
        classList={{ 'modal-panel-danger': !!props.danger }}
        role="dialog"
        aria-modal="true"
        aria-labelledby="modal-title"
        aria-describedby={props.description ? 'modal-description' : undefined}
        tabindex="-1"
        onKeyDown={(event) => event.key === 'Escape' && props.onClose()}
      >
        <div class="modal-topline" />
        <header class="modal-header">
          <div>
            <h2 id="modal-title">{props.title}</h2>
            <Show when={props.description}><p id="modal-description">{props.description}</p></Show>
          </div>
          <button class="icon-button modal-close" type="button" aria-label="Đóng hộp thoại" onClick={props.onClose}><X size={18} /></button>
        </header>
        {props.children}
      </section>
    </div>
  );
}
