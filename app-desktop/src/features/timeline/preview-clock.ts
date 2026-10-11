/** Absolute monotonic anchors avoid adding rounded RAF deltas on long projects. */
export class PreviewClock {
  private anchorMs = 0;
  private anchorUs = 0;
  private running = false;
  private revision = -1;
  private published = -1;
  advance(nowMs: number, timeUs: number, revision: number, playing: boolean, ready: boolean, boundaryUs: number): number {
    // A trim/delete can shorten the document while the transport is paused.
    timeUs = Math.min(timeUs, boundaryUs);
    if (revision !== this.revision || timeUs !== this.published || !playing || !ready) {
      this.anchorMs = nowMs; this.anchorUs = timeUs; this.running = false;
    }
    this.revision = revision;
    if (playing && ready && !this.running) { this.anchorMs = nowMs; this.anchorUs = timeUs; this.running = true; }
    const next = playing && ready ? Math.min(boundaryUs, this.anchorUs + Math.max(0, Math.round((nowMs - this.anchorMs) * 1000))) : timeUs;
    this.published = next;
    return next;
  }
}

export interface PreviewParticipant {
  /** Prepare the latest timestamp while muted/paused. False gates the shared clock. */
  prepare(timeUs: number, revision: number, playing: boolean): boolean;
  play(playing: boolean): void;
  error(): string | undefined;
}
export class PreviewPlayers {
  private players = new Map<string, PreviewParticipant>();
  register(id: string, player: PreviewParticipant) {
    this.players.set(id, player);
    return () => { player.play(false); if (this.players.get(id) === player) this.players.delete(id); };
  }
  /** Inactive prefetched players never emit sound or block active playback. */
  sync(ids: string[], timeUs: number, revision: number, playing: boolean, manifestReady = true) {
    const active = new Set(ids);
    let ready = manifestReady, error: string | undefined;
    for (const [id, player] of this.players) {
      if (!active.has(id)) { player.play(false); continue; }
      error ??= player.error();
      if (!player.prepare(timeUs, revision, playing)) ready = false;
    }
    if (ids.some(id => !this.players.has(id))) ready = false;
    ready &&= !error;
    // Gate everyone together: a stalled image/video must also stop the voices.
    for (const [id, player] of this.players) player.play(playing && ready && active.has(id));
    return { ready, error };
  }
  pause() { for (const player of this.players.values()) player.play(false); }
}
