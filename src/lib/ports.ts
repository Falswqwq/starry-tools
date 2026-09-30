/**
 * 端口类型的显示与判断。
 *
 * 这里有一小段逻辑和 Rust 侧重复了（见 `src-tauri/src/model/port_type.rs` 的
 * `accepts`）。重复是故意的：拖动连线时需要**同步**判断能不能接，没法等一次
 * IPC 往返。后端仍然会在每次改动后重新检查一遍，以那边为准。
 */

import type { PortType } from './types';

export function portKey(ty: PortType): string {
  return typeof ty === 'string' ? ty : `image:${ty.image}`;
}

const INK: Record<string, string> = {
  // 整体是黑 / 白 / 灰 + 一个主题蓝。类型不再各占一个色相，
  // 而是落在这条蓝灰梯度上：具体图像是蓝，通配和其余几种是灰。
  // 「到底是 PNG 还是 JPG」由徽标上的字说，不靠颜色猜。
  any: 'var(--type-unknown)',
  'image:any': 'var(--type-unknown)',
  'image:png': 'var(--type-image)',
  'image:jpeg': 'var(--type-image)',
  'image:gif': 'var(--type-image)',
  'image:webp': 'var(--type-image)',
  'image:bmp': 'var(--type-image)',
  'image:tiff': 'var(--type-image)',
  'image:ico': 'var(--type-image)',
  'image:qoi': 'var(--type-image)',
  'image:tga': 'var(--type-image)',
  'image:pnm': 'var(--type-image)',
  text: 'var(--type-text)',
  number: 'var(--type-number)',
  bool: 'var(--type-bool)',
};

const BADGE: Record<string, string> = {
  any: 'ANY',
  'image:any': 'IMG',
  'image:png': 'PNG',
  'image:jpeg': 'JPG',
  'image:gif': 'GIF',
  'image:webp': 'WEBP',
  'image:bmp': 'BMP',
  'image:tiff': 'TIFF',
  'image:ico': 'ICO',
  'image:qoi': 'QOI',
  'image:tga': 'TGA',
  'image:pnm': 'PNM',
  text: 'TXT',
  number: 'NUM',
  bool: 'BOOL',
};

const LABEL: Record<string, string> = {
  any: '任意值',
  'image:any': '图像',
  'image:png': 'PNG 图像',
  'image:jpeg': 'JPEG 图像',
  'image:gif': 'GIF 图像',
  'image:webp': 'WebP 图像',
  'image:bmp': 'BMP 图像',
  'image:tiff': 'TIFF 图像',
  'image:ico': 'ICO 图像',
  'image:qoi': 'QOI 图像',
  'image:tga': 'TGA 图像',
  'image:pnm': 'PNM 图像',
  text: '文本',
  number: '数字',
  bool: '布尔',
};

/** 端口徽标里用的颜色。 */
export function portInk(ty: PortType): string {
  return INK[portKey(ty)] ?? 'var(--type-unknown)';
}

export function portBadge(ty: PortType): string {
  return BADGE[portKey(ty)] ?? '?';
}

export function portLabel(ty: PortType): string {
  return LABEL[portKey(ty)] ?? portKey(ty);
}

/**
 * 编辑期能否把 `source` 接到 `target` 上。
 *
 * 与后端一致的地方：通配一头出现就放行，「格式未知的图像」可以接进任何图像端口 ——
 * 都等运行期再由实际值说话。
 */
export function accepts(target: PortType, source: PortType): boolean {
  const targetKey = portKey(target);
  const sourceKey = portKey(source);
  if (targetKey === 'any' || sourceKey === 'any') return true;
  if (targetKey === sourceKey) return true;
  if (targetKey.startsWith('image:') && sourceKey.startsWith('image:')) {
    return targetKey === 'image:any' || sourceKey === 'image:any';
  }
  return false;
}
