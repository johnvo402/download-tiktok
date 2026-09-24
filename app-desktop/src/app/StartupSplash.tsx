import Brand from '../shared/components/Brand';

export default function StartupSplash(props: { leaving: boolean }) {
  return (
    <div class="startup-splash" classList={{ 'startup-splash-leaving': props.leaving }} role="status" aria-live="polite" aria-label="Đang khởi tạo JC AI">
      <div class="startup-splash-grid" aria-hidden="true" />
      <div class="startup-splash-glow" aria-hidden="true" />
      <div class="startup-splash-content">
        <Brand />
        <div class="startup-splash-wave" aria-hidden="true"><i /><i /><i /><i /><i /><i /><i /></div>
        <div class="startup-splash-copy">
          <span>Đang chuẩn bị không gian làm việc…</span>
        </div>
        <div class="startup-splash-progress" aria-hidden="true"><span /></div>
        <small class="startup-splash-caption">LOCAL-FIRST VIDEO WORKFLOW</small>
      </div>
    </div>
  );
}
