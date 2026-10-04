"""Development experiment only; not registered as a production inpainting backend.
Exports dynamic temporal input and compares real weights with PyTorch on two window sizes.
"""
import argparse
import json
from pathlib import Path
import sys
import time
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'python'))
import numpy as np
import torch
from worker.text_removal.sttn_model import InpaintGenerator


class ExportModel(torch.nn.Module):
    def __init__(self,model):super().__init__();self.model=model
    def forward(self,images,masks):
        features=self.model.encoder(images*(1-masks))
        return torch.tanh(self.model.decoder(self.model.infer(features,masks)))


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--model',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True);p.add_argument('--openvino',action='store_true');a=p.parse_args()
    a.output.mkdir(parents=True,exist_ok=True);report={'productionEnabled':False,'cases':[]}
    torch.set_num_threads(4);torch.manual_seed(0)
    model=InpaintGenerator();state=torch.load(a.model,map_location='cpu',weights_only=True)
    model.load_state_dict(state.get('netG',state),strict=True);wrapper=ExportModel(model.eval()).eval()
    def inputs(n):
        image=torch.rand(n,3,240,432)*2-1;mask=torch.zeros(n,1,240,432);mask[:,:,175:205,90:315]=1
        return image,mask
    try:
        image,mask=inputs(2);dest=a.output/'sttn-experiment.onnx'
        with torch.inference_mode():
            torch.onnx.export(wrapper,(image,mask),dest,input_names=['images','masks'],output_names=['prediction'],
                dynamic_axes={'images':{0:'time'},'masks':{0:'time'},'prediction':{0:'time'}},
                opset_version=17,dynamo=False)
        import onnxruntime as ort
        options=ort.SessionOptions();options.intra_op_num_threads=4
        session=ort.InferenceSession(str(dest),sess_options=options,providers=['CPUExecutionProvider'])
        ov_model=None
        if a.openvino:
            import openvino as ov
            core=ov.Core();report['openvinoDevices']=core.available_devices
            if not any(d.startswith('GPU') for d in core.available_devices):raise RuntimeError('No Intel GPU available')
            ov_model=core.compile_model(str(dest),'GPU',{'INFERENCE_PRECISION_HINT':'f32'})
            report['openvinoExecutionDevices']=list(ov_model.get_property('EXECUTION_DEVICES'))
        for n in (2,3):
            image,mask=inputs(n)
            with torch.inference_mode():reference=wrapper(image,mask).numpy()
            start=time.perf_counter();actual=session.run(None,{'images':image.numpy(),'masks':mask.numpy()})[0]
            case={'frames':n,'ortSeconds':time.perf_counter()-start,'ortMaxAbsError':float(np.max(np.abs(actual-reference)))}
            if ov_model is not None:
                start=time.perf_counter();result=ov_model([image.numpy(),mask.numpy()])[0]
                case.update(openvinoSeconds=time.perf_counter()-start,openvinoMaxAbsError=float(np.max(np.abs(result-reference))))
            report['cases'].append(case)
        report['viablePrototype']=all(c['ortMaxAbsError']<.003 and c.get('openvinoMaxAbsError',0)<.01 for c in report['cases'])
    except Exception as error:
        report['viablePrototype']=False;report['error']=str(error)
    (a.output/'prototype-report.json').write_text(json.dumps(report,indent=2),encoding='utf-8');print(json.dumps(report,indent=2))

if __name__=='__main__':main()
