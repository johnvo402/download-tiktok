"""Opt in with JC_STTN_MODEL; no model download or Torch import in normal tests."""
import os
from pathlib import Path
import unittest
import numpy as np
from worker.text_removal.frames import Frame


@unittest.skipUnless(os.environ.get('JC_STTN_MODEL'),'set JC_STTN_MODEL for real STTN integration')
class SttnIntegration(unittest.TestCase):
    def test_buffer_predictions_match_legacy_forward_and_cache(self):
        from worker.text_removal.inpainting import SttnInpainter, tensor_arrays
        engine=SttnInpainter(Path(os.environ['JC_STTN_MODEL']),'cpu');engine.prepare()
        try:
            frames=[Frame(i,i/10,(i+1)/10,np.full((240,432,3),100+i*5,np.uint8)) for i in range(2)]
            masks=[np.zeros((240,432),np.uint8) for _ in frames]
            for m in masks:m[175:200,80:300]=255
            a=engine.process(frames,masks,[0],[1])
            b=engine.process(frames,masks,[0],[1])
            np.testing.assert_array_equal(a,b)
            self.assertEqual(engine.feature_hits,2);self.assertEqual(engine.feature_misses,2)
            torch=engine.torch
            with torch.inference_mode():
                images,ms=tensor_arrays([f.rgb for f in frames],masks,torch,engine.device)
                features=engine.model.encoder(images*(1-ms))
                output=((torch.tanh(engine.model.decoder(engine.model.infer(features,ms)[:1]))+1)*127.5).clamp(0,255)
                reference=output.permute(0,2,3,1).byte().cpu().numpy()
            self.assertLessEqual(np.max(np.abs(a.astype(int)-reference.astype(int))),1)
            masks[0][0:20,0:20]=255
            engine.process(frames,masks,[0],[1])
            self.assertEqual(engine.feature_misses,3)  # Mask changes invalidate encoded features.
        finally:engine.close()

    def test_cuda_runtime_when_available(self):
        import torch
        if not torch.cuda.is_available():self.skipTest('NVIDIA CUDA unavailable on this test machine')
        from worker.text_removal.inpainting import SttnInpainter
        engine=SttnInpainter(Path(os.environ['JC_STTN_MODEL']),'cuda');engine.prepare()
        try:
            f=Frame(0,0,.1,np.full((240,432,3),100,np.uint8));m=np.zeros((240,432),np.uint8);m[175:200,80:300]=255
            result=engine.process([f],[m],[0],[])
            self.assertEqual(result.shape,(1,240,432,3));self.assertEqual(engine.device.type,'cuda')
        finally:engine.close()
