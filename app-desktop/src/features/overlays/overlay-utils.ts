import type { BlurRegion, ProjectLogo, ProjectTextOverlay } from '../projects/types';

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface FittedRect extends Rect {}

const roundGeometry = (value: number) => {
  const rounded = Math.round(value * 1_000_000_000) / 1_000_000_000;
  return Object.is(rounded, -0) ? 0 : rounded;
};

export function fitContainRect(containerWidth: number, containerHeight: number, mediaWidth: number, mediaHeight: number): FittedRect {
  if (![containerWidth, containerHeight, mediaWidth, mediaHeight].every(Number.isFinite)
    || containerWidth <= 0 || containerHeight <= 0 || mediaWidth <= 0 || mediaHeight <= 0) {
    return { x: 0, y: 0, width: Math.max(containerWidth, 0), height: Math.max(containerHeight, 0) };
  }
  const scale = Math.min(containerWidth / mediaWidth, containerHeight / mediaHeight);
  const width = mediaWidth * scale;
  const height = mediaHeight * scale;
  return { x: roundGeometry((containerWidth - width) / 2), y: roundGeometry((containerHeight - height) / 2), width: roundGeometry(width), height: roundGeometry(height) };
}

export function clampRect(rect: Rect): Rect {
  const width = Math.max(0.02, Math.min(1, rect.width));
  const height = Math.max(0.02, Math.min(1, rect.height));
  return {
    x: Math.max(0, Math.min(1 - width, rect.x)),
    y: Math.max(0, Math.min(1 - height, rect.y)),
    width,
    height,
  };
}

export function moveRect(rect: Rect, dx: number, dy: number): Rect {
  return clampRect({ ...rect, x: rect.x + dx, y: rect.y + dy });
}

export function resizeRect(rect: Rect, dx: number, dy: number, normalizedAspectRatio?: number): Rect {
  let width = rect.width + dx;
  let height = rect.height + dy;
  if (normalizedAspectRatio && Number.isFinite(normalizedAspectRatio) && normalizedAspectRatio > 0) {
    if (Math.abs(dx) > Math.abs(dy)) height = width / normalizedAspectRatio;
    else width = height * normalizedAspectRatio;
  }
  width = Math.max(0.02, Math.min(width, 1 - rect.x));
  height = Math.max(0.02, Math.min(height, 1 - rect.y));
  if (normalizedAspectRatio && Math.abs(width / height - normalizedAspectRatio) > 0.005) {
    width = Math.min(width, height * normalizedAspectRatio);
    height = Math.min(height, width / normalizedAspectRatio);
  }
  return { ...rect, width: roundGeometry(width), height: roundGeometry(height) };
}

export function blurRect(region: BlurRegion): Rect {
  return { x: region.x, y: region.y, width: region.width, height: region.height };
}

export function logoRect(logo: ProjectLogo): Rect {
  return { x: logo.x, y: logo.y, width: logo.width, height: logo.height };
}

export function textOverlayRect(text: ProjectTextOverlay): Rect {
  return { x: text.x, y: text.y, width: text.width, height: text.height };
}
