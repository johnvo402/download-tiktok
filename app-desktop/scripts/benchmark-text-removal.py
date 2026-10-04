"""Opt-in real STTN comparison; never run/download weights in ordinary tests.

python scripts/benchmark-text-removal.py --model path/to/sttn.pth --output path
Writes only to --output. Synthetic clip has known clean background for quality metrics.
"""
import argparse
from fractions import Fraction
import json
from pathlib import Path
import sys
import subprocess
import time
import zlib

sys.path.insert(0, str(Path(__file__).resolve().parents[1]/'python'))
import av
import cv2
import numpy as np
from PIL import Image, ImageDraw, ImageFont
import torch
from worker.text_removal.frames import Frame, FrameSource
from worker.text_removal.inpainting import SttnInpainter, tensor_arrays
from worker.text_removal.patch_cache import atomic_json, load
from worker.text_removal.pipeline import process
from worker.text_removal.sttn_engine import process_sttn_frames, _tensor_inputs


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--model',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--phase',choices=['legacy','v2'])
    args=parser.parse_args(); root=args.output.resolve(); root.mkdir(parents=True,exist_ok=True)
    torch.set_num_threads(4)
    if args.phase:
        began=time.perf_counter()
        if args.phase=='legacy':
            result=process_sttn_frames(root/'legacy/frames',root/'legacy/masks',root/'legacy/clean',args.model,lambda e:None)
        else:
            result=process(json.loads((root/'request.json').read_text(encoding='utf-8')),lambda e:None)
        result['phaseSeconds']=time.perf_counter()-began
        import psutil
        mem=psutil.Process().memory_info()
        result['peakProcessRssBytes']=getattr(mem,'peak_wset',mem.rss)
        atomic_json(root/f'{args.phase}-phase.json',result)
        return
    if (root/'v2/result.json').exists():
        raise RuntimeError('Use a fresh --output directory for an uncached comparison')
    def phase(name):
        import psutil
        child=subprocess.Popen([sys.executable,str(Path(__file__).resolve()),'--model',str(args.model.resolve()),
                                '--output',str(root),'--phase',name])
        peak=0
        while child.poll() is None:
            try:peak=max(peak,psutil.Process(child.pid).memory_info().rss)
            except psutil.NoSuchProcess:pass
            time.sleep(.05)
        if child.returncode:raise RuntimeError(f'{name} benchmark failed: {child.returncode}')
        result=json.loads((root/f'{name}-phase.json').read_text(encoding='utf-8'))
        result['peakLauncherRssBytes']=peak
        return result
    w,h,n,fps=432,240,12,6
    source=root/'Phụ đề thử nghiệm.nut'; frames_dir=root/'legacy/frames'; masks_dir=root/'legacy/masks'
    frames_dir.mkdir(parents=True,exist_ok=True);masks_dir.mkdir(parents=True,exist_ok=True)
    font=ImageFont.truetype('C:/Windows/Fonts/arial.ttf',20)
    original=[]; clean=[]
    with av.open(str(source),'w',format='nut') as out:
        stream=out.add_stream('ffv1',rate=fps);stream.width=w;stream.height=h;stream.pix_fmt='bgr0'
        stream.time_base=stream.codec_context.time_base=Fraction(1,60000)
        for i in range(n):
            yy,xx=np.indices((h,w));base=np.stack(((xx+i*3)%256,(yy*2)%256,np.full_like(xx,80)),axis=-1).astype(np.uint8)
            cv2.rectangle(base,(i*5,25),(i*5+100,75),(200,100,60),-1)
            clean.append(base.copy());image=Image.fromarray(base);draw=ImageDraw.Draw(image)
            draw.text((100,180),'Xin chào Việt Nam!',font=font,fill='white',stroke_width=1,stroke_fill='black')
            rgb=np.array(image);original.append(rgb)
            frame=av.VideoFrame.from_ndarray(rgb,format='rgb24');frame.pts=i*10000;frame.time_base=Fraction(1,60000)
            for p in stream.encode(frame):out.mux(p)
        for p in stream.encode():out.mux(p)
    mask=np.zeros((h,w),np.uint8);mask[178:210,96:304]=255
    begin=time.perf_counter()
    for i,rgb in enumerate(original):
        Image.fromarray(rgb).save(frames_dir/f'frame_{i+1:08d}.jpg',quality=95)
        Image.fromarray(mask).save(masks_dir/f'mask_{i+1:08d}.png')
    old_extract=time.perf_counter()-begin
    # Equal buffer/file preprocessing on losslessly stored frames must be exact.
    parity=root/'parity';parity.mkdir(exist_ok=True)
    for i in range(2):
        Image.fromarray(original[i]).save(parity/f'frame_{i}.png')
        Image.fromarray(mask).save(parity/f'mask_{i}.png')
    a,b=_tensor_inputs([parity/f'frame_{i}.png' for i in range(2)],
                       [parity/f'mask_{i}.png' for i in range(2)],[0,1],torch,torch.device('cpu'))
    c,d=tensor_arrays(original[:2],[mask,mask],torch,torch.device('cpu'))
    assert torch.equal(a,c) and torch.equal(b,d),'preprocessing differs'
    print('Measuring legacy STTN in isolated process...',flush=True)
    old=phase('legacy')
    old_seconds=old['phaseSeconds'];old_peak=old['peakProcessRssBytes']
    detection=root/'detection.json'
    atomic_json(detection,{'version':2,'key':'benchmark','complete':True,'intervals':[
        {'startSeconds':0,'endSeconds':n/fps,'boxes':[[[96,178],[304,178],[304,210],[96,210]]]}]})
    request=dict(sourcePath=str(source),sourceKey='benchmark',detectionPath=str(detection),detectionKey='benchmark',
                 processingRect=dict(x=0,y=0,width=w,height=h),ocrRect=dict(x=96,y=178,width=208,height=32),
                 outputDir=str(root/'v2'),modelPath=str(args.model.resolve()),modelHash='benchmark',
                 videoHeight=h,videoDuration=n/fps,inpaintDevice='cpu',maskMode='fullRegion')
    atomic_json(root/'request.json',request)
    print('Measuring buffered STTN in isolated process...',flush=True)
    new=phase('v2');new['metrics']['peakProcessRssBytes']=new['peakProcessRssBytes']
    reused=process(request,lambda e:None,backend_factory=lambda:(_ for _ in ()).throw(AssertionError('cached model loaded')))
    meta=load(Path(new['intervalResults'][0]['path']));path=Path(new['intervalResults'][0]['path'])
    new_frames=[]
    with path.open('rb') as data:
        for f in meta['frames']:
            data.seek(f['offset']);new_frames.append(np.frombuffer(zlib.decompress(data.read(f['length'])),np.uint8).reshape(h,w,4))
    old_frames=[np.array(Image.open(p)) for p in sorted((root/'legacy/clean').glob('*.png'))]
    selection=mask>0
    def error(images):return float(np.mean([np.abs(im[...,:3][selection].astype(float)-clean[i][selection]).mean() for i,im in enumerate(images)]))
    def motion(images):return float(np.mean([np.abs(images[i][...,:3][selection].astype(float)-images[i-1][...,:3][selection]).mean() for i in range(1,len(images))]))
    grid=Image.new('RGB',(w*4,h*3))
    for row,i in enumerate([0,5,11]):
        for col,img in enumerate([original[i],clean[i],old_frames[i][...,:3],new_frames[i][...,:3]]):grid.paste(Image.fromarray(img),(col*w,row*h))
    grid.save(root/'comparison.png')
    report=dict(model=str(args.model.resolve()),device='cpu',frames=n,sourceFps=fps,preprocessingExact=True,
                legacy=dict(extractSeconds=old_extract,sttnAndWriteSeconds=old_seconds,totalSeconds=old_extract+old_seconds,
                            imageFiles=sum(1 for p in (root/'legacy').rglob('*') if p.suffix in {'.jpg','.png'}),
                            imageBytes=sum(p.stat().st_size for p in (root/'legacy').rglob('*') if p.is_file()),
                            peakProcessRssBytes=old_peak,maskedMaeVsClean=error(old_frames),temporalMae=motion(old_frames)),
                v2={**new['metrics'],'maskedMaeVsClean':error(new_frames),'temporalMae':motion(new_frames)},
                cacheReuse=reused['metrics'],cudaAvailable=torch.cuda.is_available())
    atomic_json(root/'benchmark.json',report);print(json.dumps(report,indent=2),flush=True)

if __name__=='__main__':main()
