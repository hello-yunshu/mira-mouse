// SPDX-License-Identifier: AGPL-3.0-or-later
// AttentionSurface —— 在现有组件内部安全挂载单条 Beam 的表面包装。
//
// 约定（§4.2）：
// - 不新增会改变几何尺寸的外层结构：渲染与宿主同等的单个节点；
// - 保留原 className、style、ref、事件与 ARIA（透传全部剩余 props）；
// - 仅在光束激活时补充 position: relative（宿主自身已定位时不覆盖）；
// - 不默认修改 overflow；
// - 圆角继承由内部 Beam 的 border-radius: inherit 负责。
//
// 如果宿主节点本身可修改（本仓库中的控制组件都直接可改），也可以直接在
// 节点内部插入 <AttentionBeamLayer>，两种方式效果一致。

import { forwardRef, type CSSProperties, type HTMLAttributes } from 'react';
import { AttentionBeamLayer } from './AttentionBeamLayer';
import type { AttentionBeamRequest } from './attentionTypes';

export interface AttentionSurfaceProps extends Omit<HTMLAttributes<HTMLDivElement>, 'style' | 'className'> {
  className?: string;
  style?: CSSProperties;
  /** 当前应渲染在本表面上的光束；null 表示不渲染。 */
  beam?: AttentionBeamRequest | null;
  /** Beam 视觉结束后回调（仅用于展示层的完成语义；结束仲裁由 hook 负责）。 */
  onBeamFinished?: () => void;
}

function relativePosition(position: CSSProperties['position']): CSSProperties['position'] {
  return position === 'relative' || position === 'absolute' || position === 'fixed' || position === 'sticky'
    ? position
    : 'relative';
}

export const AttentionSurface = forwardRef<HTMLDivElement, AttentionSurfaceProps>(function AttentionSurface(
  { className, style, beam, onBeamFinished, children, ...rest },
  ref,
) {
  const positioned = beam ? { ...(style ?? {}), position: relativePosition(style?.position) } : style;
  return (
    <div ref={ref} className={className} style={positioned} {...rest}>
      {beam && (
        <AttentionBeamLayer
          active
          request={beam}
          onFinished={onBeamFinished}
        />
      )}
      {children}
    </div>
  );
});