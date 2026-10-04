"""Model-free tests. All media/cache writes are isolated in TemporaryDirectory."""
from fractions import Fraction
import json
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import patch
import zlib

try:
    import av
    import numpy as np
except ImportError as error:
    raise unittest.SkipTest("Install the media fixture dependencies to run ROI integration tests") from error

from worker.text_removal.frames import Frame, FrameSource
from worker.text_removal.masks import make_mask, plan_windows, scene_ids, context_masks, MaskProfile
from worker.text_removal.patch_cache import PatchWriter, atomic_json, load, patch_video
from worker.text_removal.pipeline import process
from worker.text_removal.detection import detect, valid_detection


def video(path, times=(0, .1, .2, .3, .5, .7, .8, .9), colors=None):
    with av.open(str(path), 'w', format='nut') as out:
        stream = out.add_stream('ffv1', rate=10)
        stream.width, stream.height, stream.pix_fmt = 64, 48, 'bgr0'
        stream.time_base = Fraction(1, 1000)
        stream.codec_context.time_base = stream.time_base
        for i, t in enumerate(times):
            rgb = np.full((48, 64, 3), colors[i] if colors else i*20, np.uint8)
            frame = av.VideoFrame.from_ndarray(rgb, format='rgb24')
            frame.pts, frame.time_base = round(t*1000), Fraction(1,1000)
            for packet in stream.encode(frame): out.mux(packet)
        for packet in stream.encode(): out.mux(packet)


class FakeBackend:
    device = 'cpu'
    def prepare(self): pass
    def process(self, frames, masks, neighbors, refs):
        return np.full((len(neighbors), 240, 432, 3), 100, np.uint8)
    def close(self): pass


class RemovalV2Tests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.source = self.root/'Việt Nam.nut'
        video(self.source)
        self.rect = dict(x=0, y=0, width=64, height=48)
        self.box = [[10,20],[40,20],[40,30],[10,30]]
    def tearDown(self): self.temp.cleanup()

    def request(self):
        detection = self.root/'detection.json'
        atomic_json(detection, {'version':2, 'complete':True, 'key':'det', 'intervals':[
            {'startSeconds':.1,'endSeconds':.7,'boxes':[self.box]},
            {'startSeconds':.8,'endSeconds':1,'boxes':[self.box]}]})
        return dict(detectionPath=str(detection), detectionKey='det', sourceKey='source',
                    sourcePath=str(self.source), processingRect=self.rect, ocrRect=self.rect,
                    outputDir=str(self.root/'patches'), modelHash='model', videoHeight=48,
                    videoDuration=1, maskMode='ocrGuided')

    def test_vfr_decode_and_windows_preserve_pts_and_cover_once(self):
        with FrameSource(self.source,self.rect) as source:
            frames=list(source.frames(0,1))
        self.assertEqual([round(f.pts,3) for f in frames],[0,.1,.2,.3,.5,.7,.8,.9])
        self.assertEqual(round(frames[3].end,3),.5)
        with FrameSource(self.source,self.rect) as source:
            cores=[f.index for core,_ in source.windows(0,1) for f in core]
            self.assertEqual(cores,list(range(8)))
            self.assertLessEqual(source.peak_buffer_bytes,8*64*48*3)

    def test_rgb_and_roi_coordinates(self):
        video(self.source, (0,.1), [(255,0,0),(0,0,255)])
        with FrameSource(self.source,dict(x=10,y=15,width=30,height=20)) as source:
            _,rgb=source.at(0)
        self.assertEqual(rgb.shape,(20,30,3))
        self.assertTrue(np.all(rgb == [255,0,0]))
        with self.assertRaises(ValueError): FrameSource(self.source,dict(x=-1,y=0,width=20,height=20))

    def test_masks_are_local_and_preserve_outside_pixels(self):
        mask,alpha=make_mask(64,48,[self.box],1080)
        self.assertEqual(mask[25,25],255)
        self.assertEqual(mask[0,0],0)
        self.assertTrue(np.all(alpha[mask==0]==0))
        full,_=make_mask(64,48,[],1080,full_region=dict(x=10,y=20,width=30,height=10))
        self.assertEqual(np.count_nonzero(full),300)

    def test_scene_cuts_and_short_scenes_have_no_cross_scene_refs(self):
        frames=[Frame(i,i/10,(i+1)/10,np.full((48,64,3),v,np.uint8)) for i,v in enumerate([0,0,255,0,0])]
        scenes=scene_ids(frames)
        masks=[np.ones((48,64),np.uint8) for _ in frames]
        plans=list(plan_windows(5,[2],scenes,masks))
        self.assertEqual(len(plans),1)
        self.assertEqual(plans[0],([2],[]))

    def test_temporal_context_masks_adjacent_captions_but_not_gaps(self):
        intervals=[{'startSeconds':.1,'endSeconds':.4,'boxes':[self.box]},
                   {'startSeconds':.5,'endSeconds':.8,'boxes':[[[40,5],[55,5],[55,15],[40,15]]]}]
        frames=[Frame(i,t,t+.1,np.zeros((48,64,3),np.uint8)) for i,t in enumerate([.2,.4,.6,.9])]
        binary,_=make_mask(64,48,[self.box],48)
        masks=context_masks(frames,intervals,[.1,.5],0,binary,64,48,self.rect,48,MaskProfile(),'ocrGuided')
        self.assertGreater(masks[0][25,25],0)
        self.assertFalse(np.any(masks[1]));self.assertFalse(np.any(masks[3]))
        self.assertGreater(masks[2][10,45],0)
        self.assertEqual(masks[2][25,25],0)

    def test_pack_integrity_timestamps_and_unicode_path(self):
        path=self.root/'phụ đề.pack'; rgba=np.full((48,64,4),125,np.uint8)
        writer=PatchWriter(path,64,48,'key')
        writer.append(1.123,1.245,rgba); writer.append(1.4,1.5,rgba); writer.commit({}); writer.close()
        self.assertEqual(load(path,'key')['frames'][1]['pts'],1.4)
        self.assertIsNone(load(path,'wrong'))
        content=bytearray(path.read_bytes());content[-4]^=255;path.write_bytes(content)
        self.assertIsNone(load(path,'key'))

    def test_invalid_nan_timestamp_and_cancel_cleanup(self):
        writer=PatchWriter(self.root/'a.pack',64,48,'key')
        with self.assertRaises(ValueError): writer.append(float('nan'),1,np.zeros((48,64,4),np.uint8))
        writer.close()
        self.assertFalse(list(self.root.glob('*.partial')))
        request=self.request()
        with self.assertRaises(InterruptedError): process(request,lambda e:None,backend_factory=FakeBackend,cancel=lambda:True)
        self.assertFalse(list((self.root/'patches').glob('*.pack')))

    def test_pipeline_no_images_and_cache_resume_without_model(self):
        request=self.request()
        result=process(request,lambda e:None,backend_factory=FakeBackend)
        self.assertEqual(result['metrics']['activeFrames'],6)
        self.assertEqual(result['metrics']['temporaryImageFiles'],0)
        self.assertFalse(list(self.root.rglob('*.png'))+list(self.root.rglob('*.jpg')))
        rerun=process(request,lambda e:None,backend_factory=lambda:self.fail('cache loaded model'))
        self.assertEqual(rerun['metrics']['reusedSpans'],2)
        self.assertEqual(rerun['metrics']['decodedRoiFrames'],0)
        first=Path(result['intervalResults'][0]['path'])
        first.write_bytes(b'broken')
        repaired=process(request,lambda e:None,backend_factory=FakeBackend)
        self.assertEqual(repaired['metrics']['reusedSpans'],1)
        self.assertIsNotNone(load(first))

    def test_lossless_compatibility_movie_keeps_alpha_and_vfr(self):
        path=self.root/'a.pack';writer=PatchWriter(path,64,48,'key')
        for pts,end in ((1.123,1.2),(1.2,1.45),(1.45,1.55)):
            rgba=np.zeros((48,64,4),np.uint8);rgba[...,0]=255;rgba[20:30,10:40,3]=255
            writer.append(pts,end,rgba)
        writer.commit({});writer.close()
        with av.open(str(patch_video(path))) as movie:
            frames=list(movie.decode(video=0))
        self.assertEqual([round(float(f.pts*f.time_base),3) for f in frames],[0,.077,.327])
        np.testing.assert_array_equal(frames[0].to_ndarray(format='rgba'),rgba)

    def test_cancel_after_one_span_resumes_completed_checkpoint(self):
        request=self.request();cancelled=False
        def emit(event):
            nonlocal cancelled
            if event.get('stage')=='inpaint' and event['progress']>=.5:cancelled=True
        with self.assertRaises(InterruptedError):
            process(request,emit,backend_factory=FakeBackend,cancel=lambda:cancelled)
        out=self.root/'patches'
        self.assertEqual(len(list(out.glob('*.pack'))),1)
        self.assertFalse((out/'result.json').exists())
        (out/'terminated.partial').write_bytes(b'not committed')
        resumed=process(request,lambda e:None,backend_factory=FakeBackend)
        self.assertEqual(resumed['metrics']['reusedSpans'],1)
        self.assertFalse(list(out.glob('*.partial')))

    def test_detection_sparse_and_reused_without_ocr(self):
        def engine(*_):
            return lambda image: SimpleNamespace(boxes=np.array([self.box]),txts=np.array(['Xin chào']),scores=[.9])
        request=dict(sourcePath=str(self.source), scanRect=self.rect, startSeconds=0,endSeconds=1,
                     scanFps=2,cacheDir=str(self.root), detectionPath=str(self.root/'det.json'),detectionKey='key')
        result=detect(request,lambda e:None,engine_factory=engine)
        self.assertEqual(result['metrics']['ocrCalls'],2)
        self.assertEqual(len(result['intervals']),1)
        self.assertAlmostEqual(result['intervals'][0]['confidence'],.9)
        again=detect(request,lambda e:None,engine_factory=lambda *_:self.fail('cached detection created OCR'))
        self.assertEqual(again,result)

    def test_invalid_detection_rescans_instead_of_reporting_no_text(self):
        request=dict(sourcePath=str(self.source),scanRect=self.rect,startSeconds=0,endSeconds=1,scanFps=1,
                     cacheDir=str(self.root),detectionPath=str(self.root/'det.json'),detectionKey='key')
        bad={'version':2,'key':'key','complete':True,'intervals':'corrupt'}
        atomic_json(Path(request['detectionPath']),bad)
        self.assertFalse(valid_detection(bad,'key',0,1))
        result=detect(request,lambda e:None,engine_factory=lambda *_:lambda image:SimpleNamespace(boxes=[self.box]))
        self.assertEqual(len(result['intervals']),1)
        for update in ({'startSeconds':float('nan')},{'endSeconds':-1},{'boxes':[[[1,2]]]}):
            broken={**result,'intervals':[{**result['intervals'][0],**update}]}
            self.assertFalse(valid_detection(broken,'key',0,1))


if __name__=='__main__': unittest.main()
