import jcAiLogo from '../assets/jc-ai-logo.png';

export default function Brand(props: { compact?: boolean }) {
  return (
    <div class="brand-lockup">
      <div class="brand-mark" aria-hidden="true"><img src={jcAiLogo} alt="" /></div>
      {props.compact ? null : (
        <div class="brand-copy">
          <span class="brand-name">JC AI</span>
          <span class="brand-caption">VIDEO LOCALIZATION</span>
        </div>
      )}
    </div>
  );
}
