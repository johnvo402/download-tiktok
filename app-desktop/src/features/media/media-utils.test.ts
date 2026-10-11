import { describe, expect, it } from 'vitest';
import { filterMedia, mediaName, mediaUsage } from './media-utils';
import type { MediaAsset, TimelineClip } from '../timeline/types';

describe('media library source selection', () => {
  const video = { id: 'a', kind: 'video', relativePath: 'media/assets/a/source.mp4', metadata: { originalName: 'Chiến Tranh Việt.mp4' } } as MediaAsset;
  const image = { id: 'b', kind: 'image', relativePath: 'media/assets/b/map.png', metadata: { originalName: null } } as MediaAsset;
  it('finds Unicode display names and filters media without changing inventory', () => {
    const inventory = [video, image];
    expect(filterMedia(inventory, 'video', '  VIỆT ')).toEqual([video]);
    expect(filterMedia(inventory, 'image', 'map')).toEqual([image]);
    expect(filterMedia(inventory, 'audio', '')).toEqual([]);
    expect(inventory).toEqual([video, image]);
    expect(mediaName('D:\\Thư mục\\Video.mp4')).toBe('Video.mp4');
  });
  it('counts repeated source uses without counting another video or its generated voice', () => {
    const clips = [{ content: { kind: 'video', assetId: 'a', videoContentId: 'ca' } },
      { content: { kind: 'video', assetId: 'a', videoContentId: 'ca' } },
      { content: { kind: 'video', assetId: 'b', videoContentId: 'cb' } },
      { content: { kind: 'voice', videoContentId: 'ca' } }] as TimelineClip[];
    expect(mediaUsage('a', clips)).toBe(2);
    expect(mediaUsage('missing', clips)).toBe(0);
  });
});
