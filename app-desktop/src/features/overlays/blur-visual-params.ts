import contract from '../../shared/visual-effects.json';
import type { Rect } from './overlay-utils';

/** Source-video pixel sigma. CSS scales only once into displayed video pixels. */
export function blurVisualParams(strength: number, edgeBlend: number, rect: Rect, videoWidth: number, videoHeight: number, previewHeight: number) {
  const sigma = Math.max(1, Math.min(100, strength)) * contract.blurSigmaScale;
  const pixelWidth = rect.width * videoWidth;
  const pixelHeight = rect.height * videoHeight;
  const feather = strength >= contract.fullStrengthThreshold ? 0 : Math.min(pixelWidth, pixelHeight)
    * Math.max(0, Math.min(contract.maximumEdgeBlend, edgeBlend)) / contract.maximumEdgeBlend * contract.featherFraction;
  return { sigma, previewSigma: sigma * previewHeight / Math.max(1, videoHeight),
    featherXPercent: feather * 100 / Math.max(1, pixelWidth), featherYPercent: feather * 100 / Math.max(1, pixelHeight) };
}
