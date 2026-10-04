"""Opt-in STTN visual regression on synthetic clips with known clean backgrounds.

No model downloads. Does not pretend drawn silhouettes are a real-film quality test.
"""
import argparse
from fractions import Fraction
import json
from pathlib import Path
import sys
import time
import zlib

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'python'))
import av
import cv2
import numpy as np
from PIL import Image, ImageDraw, ImageFont
from worker.text_removal.patch_cache import atomic_json, load
from worker.text_removal.pipeline import process
from worker.text_removal.sttn_engine import process_sttn_frames


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--model',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();root=args.output.resolve();root.mkdir(parents=True,exist_ok=True)
    if (root/'quality.json').exists():raise RuntimeError('Use a fresh output directory')
    latin=ImageFont.truetype('C:/Windows/Fonts/arial.ttf',20)
    cjk=ImageFont.truetype('C:/Windows/Fonts/msyh.ttc',20)
    w,h,n,fps=432,240,3,6
    cases=['static','moving','camera-pan','moving-silhouette','two-line-Chinese','outline-shadow','scene-cut']
    grid=Image.new('RGB',(4*w,h*len(cases)))
    reports=[]
    for row,name in enumerate(cases):
        print(f'Testing {name}...',flush=True)
        directory=root/name;directory.mkdir(exist_ok=True)
        frames=directory/'frames';masks=directory/'masks';frames.mkdir(exist_ok=True);masks.mkdir(exist_ok=True)
        clean=[];original=[];mask=np.zeros((h,w),np.uint8);mask[154:224,68:364]=255
        source=directory/'source.nut'
        with av.open(str(source),'w',format='nut') as out:
            stream=out.add_stream('ffv1',rate=fps);stream.width=w;stream.height=h;stream.pix_fmt='bgr0'
            stream.time_base=stream.codec_context.time_base=Fraction(1,60000)
            for i in range(n):
                yy,xx=np.indices((h,w));shift=i*8 if name in ['moving','camera-pan'] else 0
                base=np.stack(((xx+shift)%256,(yy*2+shift)%256,np.full_like(xx,80)),axis=-1).astype(np.uint8)
                if name=='static':base[:]=[75,110,145]
                if name=='moving-silhouette':
                    cv2.circle(base,(200+i*10,100),35,(100,60,50),-1)
                    cv2.rectangle(base,(155+i*10,130),(245+i*10,239),(170,80,40),-1)
                if name=='scene-cut' and i>=1:base[:]=[180,210,130]
                clean.append(base.copy());im=Image.fromarray(base);draw=ImageDraw.Draw(im)
                text='我带着袁英修为回来了\n这是中文字幕' if name=='two-line-Chinese' else 'Xin chào Việt Nam!'
                font=cjk if name=='two-line-Chinese' else latin
                if name=='outline-shadow':draw.multiline_text((101,172),text,font=font,fill='black',stroke_width=3,stroke_fill='black')
                draw.multiline_text((96,166),text,font=font,fill='white',stroke_width=1,stroke_fill='black')
                rgb=np.array(im);original.append(rgb)
                Image.fromarray(rgb).save(frames/f'frame_{i+1:08d}.jpg',quality=95)
                Image.fromarray(mask).save(masks/f'mask_{i+1:08d}.png')
                frame=av.VideoFrame.from_ndarray(rgb,format='rgb24');frame.pts=i*10000;frame.time_base=Fraction(1,60000)
                for packet in stream.encode(frame):out.mux(packet)
            for packet in stream.encode():out.mux(packet)
        began=time.perf_counter()
        old=process_sttn_frames(frames,masks,directory/'legacy',args.model,lambda e:None)
        old_seconds=time.perf_counter()-began
        detection=directory/'detection.json'
        atomic_json(detection,{'version':2,'key':name,'complete':True,'intervals':[
            {'startSeconds':0,'endSeconds':n/fps,'boxes':[[[68,154],[364,154],[364,224],[68,224]]]}]})
        request=dict(sourcePath=str(source),sourceKey=name,detectionPath=str(detection),detectionKey=name,
                     processingRect=dict(x=0,y=0,width=w,height=h),ocrRect=dict(x=68,y=154,width=296,height=70),
                     outputDir=str(directory/'v2'),modelPath=str(args.model.resolve()),modelHash='real-STTN-QA',
                     videoHeight=h,videoDuration=n/fps,inpaintDevice='cpu',maskMode='fullRegion')
        result=process(request,lambda e:None)
        path=Path(result['intervalResults'][0]['path']);metadata=load(path);new_frames=[]
        with path.open('rb') as data:
            for f in metadata['frames']:
                data.seek(f['offset']);new_frames.append(np.frombuffer(zlib.decompress(data.read(f['length'])),np.uint8).reshape(h,w,4)[...,:3])
        old_frames=[np.array(Image.open(p))[...,:3] for p in sorted((directory/'legacy').glob('*.png'))]
        assert len(new_frames)==len(old_frames)==n
        select=mask>0
        def error(images):return float(np.mean([np.abs(im[select].astype(float)-clean[i][select]).mean() for i,im in enumerate(images)]))
        for col,im in enumerate([original[1],clean[1],old_frames[1],new_frames[1]]):grid.paste(Image.fromarray(im),(col*w,row*h))
        reports.append(dict(case=name,legacySeconds=old_seconds,v2Seconds=result['metrics']['elapsedSeconds'],
                            legacyMaskedMae=error(old_frames),v2MaskedMae=error(new_frames),frames=n))
    grid.save(root/'comparison.png')
    report=dict(syntheticOnly=True,device='cpu',columns=['input','known clean','legacy','v2'],cases=reports)
    atomic_json(root/'quality.json',report);print(json.dumps(report,indent=2),flush=True)

if __name__=='__main__':main()
