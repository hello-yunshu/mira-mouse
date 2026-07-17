# Mira Mouse 浮层与毛玻璃层级问题完整修改方案

> 适用仓库：`hello-yunshu/mira-mouse`  
> 目标平台：macOS 优先，同时兼容 Windows、Linux 与 Web Preview  
> 目标：一次性修复 DPI、回报率及其他弹窗的局部无毛玻璃、浮层被遮挡、提示气泡被裁切、多个浮层层级混乱等问题。

---

## 1. 问题结论

当前问题不是 macOS 原生 Vibrancy 只在某块区域失效，而是 **React DOM 挂载位置与 CSS 层叠上下文冲突**。

控制区结构大致为：

```text
control-stage-layers
├── shared-control-surface       z-index: 0
├── control-stage-content       z-index: 1
│   ├── 普通控件
│   └── FieldEditModal
├── shared-control-context       z-index: 2
└── shared-control-metric        z-index: 3
```

`FieldEditModal` 虽然设置了 `position: fixed` 和较大的 `z-index`，但它仍位于 `.control-stage-content` 创建的层叠上下文内，无法覆盖外部兄弟层中的：

- DPI 数字和单位；
- 回报率图标、标题、数字和单位；
- 部分共享动画表面；
- 控制区提示消息；
- 某些更高层的页面元素。

macOS 使用透明 Tauri 窗口、原生 Vibrancy 和 WKWebView 合成层，因此这种错误会表现得格外明显，看起来像毛玻璃中间出现局部“漏区”。

---

## 2. 修改目标

本次修改应同时完成：

1. 修复 DPI 编辑弹窗局部无毛玻璃问题。
2. 修复回报率编辑弹窗局部无毛玻璃问题。
3. 一并修复灯光颜色、普通插件字段、状态字段等所有 `FieldEditModal` 的同类问题。
4. 统一设备详情、电量统计、未来日志查看器等模态窗口的挂载方式。
5. 统一 Tooltip、Popover、Modal、Toast 的层级规则。
6. 避免浮层被卡片、滚动容器、动画层或 `overflow` 裁切。
7. 避免模态窗口打开后，背景 Tooltip、Popover 或可点击通知继续响应。
8. 保持现有界面风格、毛玻璃强度、动画语言和窗口默认尺寸不变。
9. 不修改插件协议、设备读取逻辑、Mutation 逻辑以及 UI 与插件解耦结构。
10. 不为 DPI、回报率单独编写硬编码补丁。

---

## 3. 建议修改文件

```text
src/App.tsx
src/BatteryUsage.tsx
src/Tooltip.tsx
src/styles.css
```

建议新增：

```text
src/overlay/OverlayPortal.tsx
src/overlay/Modal.tsx
src/overlay/overlayStack.ts
src/overlay/index.ts
```

建议新增测试：

```text
src/overlay/OverlayPortal.test.tsx
src/overlay/Modal.test.tsx
src/App.overlay.test.tsx
```

---

# 4. 总体架构

## 4.1 所有全屏模态窗口统一 Portal

所有全屏遮罩和模态内容必须通过 React Portal 挂到顶层容器：

```html
<body>
  <div id="root"></div>
  <div id="mira-overlay-root"></div>
</body>
```

禁止继续将全屏模态窗口直接挂在：

- `.control-stage-content`；
- `.dashboard`；
- `.settings-page`；
- `.about-page`；
- `.card`；
- 带 `transform` 的节点；
- 带 `overflow` 的节点；
- 带独立 `z-index` 的业务节点；
- 带 `filter`、`backdrop-filter`、`contain` 或 `isolation` 的节点。

## 4.2 建立统一层级 Token

替换零散的层级数字：

```css
:root {
  --z-base: 0;
  --z-sticky: 10;
  --z-popover: 100;
  --z-tooltip: 120;
  --z-modal-backdrop: 200;
  --z-modal-content: 210;
  --z-toast: 240;
  --z-system-overlay: 300;
}
```

语义建议：

| 类型 | 层级 |
|---|---:|
| 页面普通内容 | 0 |
| 页面固定内容 | 10 |
| 下拉菜单、Popover | 100 |
| Tooltip | 120 |
| Modal 遮罩 | 200 |
| Modal 内容 | 210 |
| Toast / 通知 | 240 |
| 极少数系统覆盖层 | 300 |

注意：数字不是核心修复。核心是所有全局浮层进入同一个顶层层叠上下文。

---

# 5. 新增 OverlayPortal

## 5.1 `src/overlay/OverlayPortal.tsx`

```tsx
// SPDX-License-Identifier: AGPL-3.0-or-later

import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';

const OVERLAY_ROOT_ID = 'mira-overlay-root';

function ensureOverlayRoot(): HTMLElement {
  const existing = document.getElementById(OVERLAY_ROOT_ID);
  if (existing) return existing;

  const root = document.createElement('div');
  root.id = OVERLAY_ROOT_ID;
  root.dataset.miraOverlayRoot = 'true';
  document.body.appendChild(root);
  return root;
}

export function OverlayPortal({ children }: { children: React.ReactNode }) {
  const [container, setContainer] = useState<HTMLElement | null>(null);

  useEffect(() => {
    const root = ensureOverlayRoot();
    setContainer(root);

    return () => {
      if (root.childElementCount === 0) root.remove();
    };
  }, []);

  if (!container) return null;
  return createPortal(children, container);
}
```

要求：

- 不要给每个业务弹窗创建不同 Portal Root；
- Modal、Tooltip、后续 Popover 尽量共用同一 Overlay Root；
- 不要在模块加载阶段直接访问 `document`，避免测试环境报错。

## 5.2 Overlay Root CSS

```css
#mira-overlay-root {
  position: fixed;
  inset: 0;
  z-index: var(--z-system-overlay);
  pointer-events: none;
  overflow: visible;
  isolation: isolate;
}

#mira-overlay-root > * {
  pointer-events: auto;
}
```

不要给它添加：

```css
transform
filter
backdrop-filter
opacity
contain: paint
overflow: hidden
```

这些属性可能改变合成层、毛玻璃采样范围或裁切子浮层。

---

# 6. 新增统一 Modal

## 6.1 `src/overlay/Modal.tsx`

```tsx
// SPDX-License-Identifier: AGPL-3.0-or-later

import { useEffect, useId, useLayoutEffect, useRef } from 'react';
import { OverlayPortal } from './OverlayPortal';

interface ModalProps {
  open: boolean;
  title?: string;
  ariaLabel?: string;
  size?: 'small' | 'medium' | 'large';
  className?: string;
  backdropClassName?: string;
  closeOnBackdrop?: boolean;
  closeOnEscape?: boolean;
  onClose: () => void;
  children: React.ReactNode;
}

function getFocusable(container: HTMLElement): HTMLElement[] {
  const selector = [
    'button:not([disabled])',
    'a[href]',
    'input:not([disabled])',
    'select:not([disabled])',
    'textarea:not([disabled])',
    '[tabindex]:not([tabindex="-1"])',
  ].join(',');

  return Array.from(container.querySelectorAll<HTMLElement>(selector))
    .filter((element) => !element.hasAttribute('hidden'));
}

export function Modal({
  open,
  title,
  ariaLabel,
  size = 'medium',
  className,
  backdropClassName,
  closeOnBackdrop = true,
  closeOnEscape = true,
  onClose,
  children,
}: ModalProps) {
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);

  useLayoutEffect(() => {
    if (!open) return;

    previousFocusRef.current = document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null;

    const frame = requestAnimationFrame(() => {
      const dialog = dialogRef.current;
      if (!dialog) return;
      const focusable = getFocusable(dialog);
      (focusable[0] ?? dialog).focus({ preventScroll: true });
    });

    return () => cancelAnimationFrame(frame);
  }, [open]);

  useEffect(() => {
    if (!open) return;

    const appRoot = document.getElementById('root');
    appRoot?.setAttribute('inert', '');
    appRoot?.setAttribute('aria-hidden', 'true');

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && closeOnEscape) {
        event.preventDefault();
        onClose();
        return;
      }

      if (event.key !== 'Tab') return;

      const dialog = dialogRef.current;
      if (!dialog) return;

      const focusable = getFocusable(dialog);
      if (focusable.length === 0) {
        event.preventDefault();
        dialog.focus();
        return;
      }

      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement;

      if (event.shiftKey && active === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      }
    };

    document.addEventListener('keydown', handleKeyDown);

    return () => {
      document.removeEventListener('keydown', handleKeyDown);
      appRoot?.removeAttribute('inert');
      appRoot?.removeAttribute('aria-hidden');

      requestAnimationFrame(() => {
        previousFocusRef.current?.focus?.({ preventScroll: true });
      });
    };
  }, [closeOnEscape, onClose, open]);

  if (!open) return null;

  return (
    <OverlayPortal>
      <div
        className={['modal-backdrop', backdropClassName].filter(Boolean).join(' ')}
        data-modal-size={size}
        onMouseDown={(event) => {
          if (closeOnBackdrop && event.target === event.currentTarget) onClose();
        }}
      >
        <div
          ref={dialogRef}
          className={['modal-surface', className].filter(Boolean).join(' ')}
          role="dialog"
          aria-modal="true"
          aria-labelledby={title ? titleId : undefined}
          aria-label={!title ? ariaLabel : undefined}
          tabIndex={-1}
        >
          {title && (
            <span id={titleId} className="sr-only">
              {title}
            </span>
          )}
          {children}
        </div>
      </div>
    </OverlayPortal>
  );
}
```

## 6.2 Modal CSS

```css
.modal-backdrop {
  position: fixed;
  inset: 0;
  z-index: var(--z-modal-backdrop);
  display: grid;
  place-items: center;
  padding: var(--modal-backdrop-padding);
  background: var(--modal-backdrop-bg);
  -webkit-backdrop-filter: var(--modal-backdrop-blur);
  backdrop-filter: var(--modal-backdrop-blur);
  pointer-events: auto;
  animation: mira-scrim-enter var(--motion-base) ease-out backwards;
}

.modal-surface {
  position: relative;
  z-index: var(--z-modal-content);
  max-width: calc(100vw - 32px);
  max-height: calc(100vh - 32px);
  border: 1px solid var(--floating-glass-border);
  background: var(--floating-glass-bg);
  -webkit-backdrop-filter: var(--floating-glass-blur);
  backdrop-filter: var(--floating-glass-blur);
  box-shadow: var(--floating-glass-shadow);
  animation: mira-modal-enter var(--motion-slow) var(--motion-ease-out) backwards;
  transform-origin: 50% 42%;
}

.modal-backdrop[data-modal-size="small"] .modal-surface {
  width: min(304px, calc(100vw - 32px));
  border-radius: var(--modal-radius-md);
}

.modal-backdrop[data-modal-size="medium"] .modal-surface {
  width: min(468px, calc(100vw - 32px));
  border-radius: var(--modal-radius-lg);
}

.modal-backdrop[data-modal-size="large"] .modal-surface {
  width: min(620px, calc(100vw - 32px));
  border-radius: var(--modal-radius-lg);
}

.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}
```

---

# 7. 修改 EditModal：核心修复

## 7.1 修改 `src/App.tsx`

导入：

```tsx
import { Modal } from './overlay';
```

将原 `EditModal` 改为：

```tsx
function EditModal({
  title,
  children,
  submitLabel = i18n.t('common.apply'),
  submitDisabled,
  onClose,
  onSubmit,
}: EditModalProps) {
  return (
    <Modal
      open
      title={title}
      size="small"
      className="edit-modal"
      backdropClassName="edit-modal-backdrop"
      onClose={onClose}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit();
        }}
      >
        <header>
          <h3>{title}</h3>
        </header>

        <div className="edit-modal-body">{children}</div>

        <footer>
          <button type="button" className="secondary" onClick={onClose}>
            {i18n.t('common.cancel')}
          </button>
          <button type="submit" disabled={submitDisabled}>
            {submitLabel}
          </button>
        </footer>
      </form>
    </Modal>
  );
}
```

要求：

- 删除 `EditModal` 自己的 Escape 监听；
- Escape、遮罩点击、焦点恢复统一交给 `Modal`；
- 不需要分别修改每个 `FieldEditModal` 调用点；
- 所有 DPI、回报率、灯光和普通插件字段会自动继承修复。

## 7.2 样式迁移

删除 `.edit-modal-backdrop` 中以下职责：

```css
position
inset
z-index
background
backdrop-filter
display
place-items
animation
```

这些统一由 `.modal-backdrop` 提供。

`.edit-modal` 最终只保留业务布局：

```css
.edit-modal {
  max-height: 100%;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  padding: var(--modal-padding);
}

.edit-modal > form {
  min-height: 0;
  display: flex;
  flex-direction: column;
}
```

---

# 8. 修改设备详情弹窗

设备详情当前通常能正常覆盖，但仍直接挂在 `.dashboard` 内，未来一旦父级新增 `transform`、`contain`、`overflow` 或独立层级，就可能再次出问题。

改为：

```tsx
function DeviceDetails({ device, onClose }: { device: DeviceState; onClose: () => void }) {
  // 原有 groups 计算保持不变

  return (
    <Modal
      open
      title={i18n.t('dashboard.allReadInfo')}
      size="medium"
      className="device-details"
      backdropClassName="details-backdrop"
      onClose={onClose}
    >
      <section aria-labelledby="device-details-title">
        <header>
          <div>
            <p className="eyebrow">{i18n.t('dashboard.readonlyReport')}</p>
            <h2 id="device-details-title">{i18n.t('dashboard.allReadInfo')}</h2>
          </div>
          <button
            className="icon-button"
            onClick={onClose}
            aria-label={i18n.t('dashboard.closeDeviceDetails')}
          >
            <X weight="regular" />
          </button>
        </header>

        {/* 原有内容保持不变 */}
      </section>
    </Modal>
  );
}
```

删除 `DeviceDetails` 自己的 Escape 监听。

---

# 9. 修改电量统计弹窗

## 9.1 使用统一 Modal

在 `src/BatteryUsage.tsx` 中，将三个分支中的：

```tsx
<div className="battery-usage-modal-overlay" onClick={onClose}>
  <div className="battery-usage-modal" onClick={(e) => e.stopPropagation()}>
```

改为：

```tsx
<Modal
  open={open}
  title={t('batteryUsage.title')}
  size="large"
  className="battery-usage-modal"
  backdropClassName="battery-usage-modal-overlay"
  onClose={onClose}
>
```

功能关闭、不支持电量、正常页面三个分支必须统一。

## 9.2 调整滚动结构

不要让整个 `.battery-usage-modal` 承担滚动，推荐：

```tsx
<div className="battery-usage-modal-layout">
  <header className="battery-usage-header">...</header>
  <div className="battery-usage-scroll-region">...</div>
</div>
```

```css
.battery-usage-modal {
  max-height: calc(100vh - 54px);
  overflow: hidden;
  padding: var(--modal-padding);
}

.battery-usage-modal-layout {
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.battery-usage-scroll-region {
  min-height: 0;
  overflow-y: auto;
  overscroll-behavior: contain;
  scrollbar-width: none;
}
```

这样标题和关闭按钮不会跟随内容滚动。

---

# 10. 修复 OverflowTip

当前 `OverflowTip` 直接作为文本容器的子元素渲染，并使用绝对定位。只要祖先存在 `overflow: hidden` 或 `overflow-y: auto`，它仍会被裁切。

不要继续维护第二套 Tooltip，改为复用现有统一 Tooltip：

```tsx
function OverflowTip({
  text,
  className,
  multiline,
}: {
  text: string;
  className?: string;
  multiline?: boolean;
}) {
  const ref = useRef<HTMLSpanElement>(null);
  const [overflowed, setOverflowed] = useState(false);

  const checkOverflow = useCallback(() => {
    const element = ref.current;
    if (!element) return;

    setOverflowed(
      multiline
        ? element.scrollHeight > element.clientHeight
        : element.scrollWidth > element.clientWidth,
    );
  }, [multiline]);

  useEffect(() => {
    checkOverflow();
    const observer = new ResizeObserver(checkOverflow);
    if (ref.current) observer.observe(ref.current);
    return () => observer.disconnect();
  }, [checkOverflow]);

  const content = <span ref={ref} className={className}>{text}</span>;
  return overflowed ? <Tooltip label={text}>{content}</Tooltip> : content;
}
```

删除旧的：

```css
.overflow-tip
.overflow-tip-host
@keyframes overflow-tip-in
@keyframes overflow-tip-in-margin
```

只保留文字本身的单行或多行截断样式。

---

# 11. 修改 Tooltip

## 11.1 Portal 目标统一

当前 Tooltip 已经使用 Portal，方向正确。将直接挂载 `document.body` 改为统一 `OverlayPortal`：

```tsx
<OverlayPortal>
  <span
    role="tooltip"
    id={id}
    ref={tooltipRef}
    className="tooltip-content"
    data-show={visible ? 'true' : 'false'}
    style={pos ? { top: `${pos.top}px`, left: `${pos.left}px` } : undefined}
  >
    {label}
  </span>
</OverlayPortal>
```

## 11.2 模态窗口打开时关闭背景 Tooltip

新增 `src/overlay/overlayStack.ts`：

```ts
// SPDX-License-Identifier: AGPL-3.0-or-later

type Listener = () => void;

let modalCount = 0;
const listeners = new Set<Listener>();

function emit() {
  for (const listener of listeners) listener();
}

export function openModalLayer() {
  modalCount += 1;
  if (modalCount > 1) {
    console.warn('[Mira Overlay] Multiple modal layers are open.');
  }
  emit();

  return () => {
    modalCount = Math.max(0, modalCount - 1);
    emit();
  };
}

export function hasOpenModal() {
  return modalCount > 0;
}

export function subscribeOverlayStack(listener: Listener) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
```

在 `Modal` 中：

```tsx
useEffect(() => {
  if (!open) return;
  return openModalLayer();
}, [open]);
```

Tooltip 中：

```tsx
useEffect(() => {
  return subscribeOverlayStack(() => {
    if (!hasOpenModal()) return;
    setVisible(false);
    setMounted(false);
    setPos(null);
  });
}, []);
```

---

# 12. Popover 处理

当前设备切换菜单、电量 Popover、电量设备菜单等仍是业务节点内的绝对定位浮层。

本次分两阶段处理。

## 第一阶段：必须完成

- 打开 Modal 时关闭所有已打开 Popover；
- Modal 开启期间禁止重新打开背景 Popover；
- Popover 不允许位于 Modal 遮罩上方；
- 外部点击和 Escape 行为保持一致。

## 第二阶段：后续可做

建立统一 `Popover`，支持：

```tsx
<Popover
  anchorRef={triggerRef}
  placement="bottom-start"
  collisionPadding={8}
>
  ...
</Popover>
```

本次不要为了修复毛玻璃问题引入大型定位库，避免工程量失控。

---

# 13. App Notification 规则

通知可以位于 Modal 上方，但必须规范交互。

模态窗口打开时建议：

- 成功通知：可以显示；
- 错误通知：可以显示；
- 纯信息通知：可以显示；
- 带页面跳转的通知：显示但不可点击；
- 会打开另一 Modal 的通知：不可触发；
- 重启应用通知：可以保留，但点击前先关闭当前 Modal 或明确确认。

层级：

```css
.app-notification {
  z-index: var(--z-toast);
}
```

不要只依靠 CSS，建议 React 中加入：

```tsx
const notificationActionEnabled = !hasOpenModal();
```

---

# 14. macOS 特别注意事项

## 14.1 不要修改原生 Vibrancy

本次不要修改：

```rust
apply_vibrancy(...)
NSVisualEffectMaterial::Sidebar
```

也不要修改：

```json
"transparent": true
```

问题根源不是原生背景配置。

## 14.2 遮罩只做 opacity 动画

```css
@keyframes mira-scrim-enter {
  from { opacity: 0; }
  to { opacity: 1; }
}
```

不要对 `.modal-backdrop` 做：

```css
transform
filter
scale
translateZ
```

## 14.3 弹窗内容可保留轻微位移动画

```css
@keyframes mira-modal-enter {
  from {
    opacity: 0;
    transform: translateY(5px) scale(.985);
  }
  to {
    opacity: 1;
    transform: translateY(0) scale(1);
  }
}
```

如果 macOS 实机仍出现合成异常，将其改为 `opacity + margin-top`，减少额外合成层。

---

# 15. CSS 清理要求

完成迁移后，以下业务类不再分别维护全屏遮罩：

```css
.edit-modal-backdrop
.details-backdrop
.battery-usage-modal-overlay
```

它们可保留类名用于测试和个性化，但以下样式统一由 `.modal-backdrop` 提供：

```text
position
inset
z-index
background
backdrop-filter
display
place-items
padding
animation
```

以下由 `.modal-surface` 提供：

```text
border
background
backdrop-filter
box-shadow
基础圆角
基础尺寸限制
入场动画
```

业务类只保留内部布局与具体尺寸差异。

---

# 16. 可访问性要求

统一 Modal 必须具备：

1. `role="dialog"`；
2. `aria-modal="true"`；
3. 标题关联 `aria-labelledby` 或 `aria-label`；
4. 打开后焦点进入弹窗；
5. Tab 焦点限制在弹窗内；
6. Escape 关闭；
7. 关闭后焦点回到触发按钮；
8. 背景应用 `inert`；
9. 背景应用 `aria-hidden="true"`；
10. 背景按钮不能继续接受键盘操作。

不要只使用 `pointer-events: none`，因为它无法阻止键盘焦点进入背景。

---

# 17. 测试方案

## 17.1 单元测试

### OverlayPortal

验证：

- 自动创建 `#mira-overlay-root`；
- 子节点挂载到 Overlay Root；
- 多个浮层不会重复创建 Root；
- 卸载后无残留。

### Modal

验证：

- `open=false` 不渲染；
- `open=true` 渲染到 Overlay Root；
- Escape 调用 `onClose`；
- 点击遮罩调用 `onClose`；
- 点击内容不关闭；
- 打开后焦点进入弹窗；
- 关闭后焦点回到触发按钮；
- Tab 不逃出弹窗；
- 背景 root 设置 `inert`；
- 卸载后清理 `inert` 与 `aria-hidden`。

### Tooltip

验证：

- Tooltip 挂载到 Overlay Root；
- Modal 打开后 Tooltip 自动关闭；
- resize / scroll 后重新定位；
- 不被卡片或滚动容器裁切。

## 17.2 DPI 场景

1. 打开 DPI 编辑弹窗；
2. DPI 数字和单位必须被完整遮罩与模糊；
3. DPI 档位圆点和数字不得出现在遮罩上方；
4. 控制区提示消息不得出现在遮罩上方；
5. 背景控件不可点击。

## 17.3 回报率场景

1. 打开回报率编辑弹窗；
2. 波形图标必须进入遮罩；
3. “当前回报率”标题必须进入遮罩；
4. 回报率数字与 Hz 单位必须进入遮罩；
5. 模式切换按钮不可穿透点击。

## 17.4 灯光与通用字段

逐一测试：

- `modal-select`；
- `modal-color`；
- `modal-range`；
- `modal-number`；
- `modal-gradient`；
- 灯光区域颜色编辑；
- 状态栏触发的字段编辑。

## 17.5 设备详情

验证：

- Dashboard 全部进入遮罩；
- 顶部导航进入遮罩；
- 设备 Hero 进入遮罩；
- 共享动画层进入遮罩；
- 关闭后焦点回到详情按钮。

## 17.6 电量统计

验证：

- 位于 Dashboard、设置页、关于页之上；
- 标题和关闭按钮不随内容滚动；
- Tooltip 不被滚动区域裁切；
- 设备切换菜单不越过 Modal 层级；
- 不允许通过通知再次打开第二个 Modal。

---

# 18. macOS 实机测试矩阵

至少测试：

- 浅色模式；
- 深色模式；
- 系统主题自动切换；
- 窗口聚焦；
- 窗口失焦后重新聚焦；
- Apple Silicon；
- Intel Mac（有条件时）；
- 不同壁纸；
- 显示缩放；
- “降低透明度”辅助功能开启与关闭。

重点观察：

- 是否还有局部毛玻璃漏区；
- 遮罩是否突然变黑；
- 打开弹窗是否闪烁；
- 动画期间是否出现一帧透明；
- 原生 Vibrancy 是否仍正常；
- 背景文字是否异常清晰。

---

# 19. Windows 与 Web Preview 验证

## Windows

- Acrylic / Mica 正常；
- 最小化、关闭按钮进入遮罩且不可穿透；
- WebView2 下 `backdrop-filter` 正常；
- Windows Preview 视觉正常。

## Web Preview

- Portal 根节点正常创建；
- 不依赖 Tauri API；
- CSS 毛玻璃正常；
- 测试环境没有 `document` 初始化报错；
- Tooltip 与 Modal 层级一致。

---

# 20. 验收标准

## 视觉

- DPI 弹窗不再出现局部无毛玻璃；
- 回报率图标、标题和数字不再保持清晰；
- 灯光及普通字段弹窗不存在同类问题；
- 所有 Modal 完整覆盖窗口内容；
- 毛玻璃强度和现有风格一致；
- 不改变主窗口默认尺寸；
- 不改变整体视觉语言。

## 交互

- Escape 关闭当前 Modal；
- 点击遮罩关闭允许关闭的 Modal；
- 点击内容不误关闭；
- Modal 打开时背景不可点击；
- Modal 打开时背景不可聚焦；
- 关闭后焦点回到触发按钮；
- 同一时间不允许叠加两个业务 Modal。

## 架构

- 所有 Modal 统一使用 Portal；
- Tooltip 统一使用 Overlay Root；
- `FieldEditModal` 不再被控制区层叠上下文限制；
- 不使用 DPI 或回报率专用层级补丁；
- 不使用 `z-index: 999999` 作为最终修复；
- 不修改插件协议、设备读取和 Mutation；
- 不破坏 UI 与插件解耦约定。

## 质量检查

以下命令应全部通过：

```bash
npm run typecheck
npm run lint
npm run test
npm run build
npm run check:quick
```

如果某项依赖本地硬件或特定环境，应在交付说明中明确未执行原因。

---

# 21. 推荐实施顺序

## 第一阶段：核心修复

1. 新增 `OverlayPortal`；
2. 新增统一 `Modal`；
3. `EditModal` 改用 Portal；
4. 验证 DPI 与回报率；
5. macOS 实机复测。

## 第二阶段：统一现有 Modal

1. 迁移 `DeviceDetails`；
2. 迁移 `BatteryUsageModal`；
3. 删除重复遮罩 CSS；
4. 统一动画和尺寸 Token。

## 第三阶段：统一 Tooltip

1. Tooltip 改用 Overlay Root；
2. Modal 打开时关闭背景 Tooltip；
3. `OverflowTip` 复用 Tooltip；
4. 验证滚动与边缘定位。

## 第四阶段：健壮性

1. 增加 Overlay Stack；
2. 禁止多 Modal 叠加；
3. 规范通知行为；
4. 增加自动化测试；
5. 增加开发期层级警告。

---

# 22. 禁止采用的修复方式

## 22.1 只增加 z-index

```css
.edit-modal-backdrop {
  z-index: 999999;
}
```

无效原因：它仍被父级层叠上下文限制。

## 22.2 打开弹窗时只隐藏 DPI 数字

```css
.control-stage.modal-open .shared-control-metric {
  visibility: hidden;
}
```

这只是掩盖问题，回报率图标、提示消息和未来动画层仍可能漏出。

## 22.3 临时提升 `.control-stage-content`

```css
.control-stage-content:has(.edit-modal-backdrop) {
  z-index: 10;
}
```

会改变整个控制区排序，且没有解决其他页面和未来弹窗。

## 22.4 修改 macOS Vibrancy 或关闭透明窗口

这会破坏整体视觉，却无法解决 WebView 内部浮层结构错误。

---

# 23. AI IDE 执行要求

1. 先阅读当前 `App.tsx`、`BatteryUsage.tsx`、`Tooltip.tsx` 和 `styles.css`。
2. 不重写业务逻辑。
3. 不修改设备读取、插件声明、Mutation 或 Tauri 后端。
4. 新增统一 Overlay Root 和 Modal 基础组件。
5. 优先把 `EditModal` Portal 化。
6. 让所有 `FieldEditModal` 自动继承修复，不分别写特例。
7. 再迁移设备详情和电量统计。
8. 复用当前设计 Token，不改变视觉风格。
9. 保留现有动画语言，但避免遮罩根节点使用 transform。
10. 打开 Modal 时让应用主体进入 `inert`。
11. 增加焦点锁定与焦点恢复。
12. Tooltip 必须挂到统一 Overlay Root。
13. `OverflowTip` 应复用统一 Tooltip。
14. 明确 Toast 与 Modal 的层级和点击规则。
15. 增加测试覆盖。
16. 完成后运行所有质量检查。
17. 输出修改文件、关键差异、测试结果以及仍需实机验证的内容。

---

# 24. 最终预期结构

```text
App
├── app-shell
│   ├── nav
│   ├── dashboard / settings / about
│   └── 普通页面内容
│
└── mira-overlay-root
    ├── popover layer
    ├── tooltip layer
    ├── modal backdrop
    │   └── modal content
    └── toast layer
```

所有全局浮层不再依赖业务组件所在位置的层叠上下文。

---

# 25. 最终结论

本问题应作为 **浮层架构问题** 处理，而不是 DPI 或回报率的局部样式问题。

最关键的修改是：

```text
让 EditModal 脱离 control-stage-content，
通过 React Portal 挂载到统一 Overlay Root。
```

完成后：

- DPI 数字和单位会正确进入毛玻璃遮罩；
- 回报率图标、标题、数字和单位会正确进入遮罩；
- 灯光及所有通用字段编辑弹窗同步修复；
- 设备详情、电量统计和未来日志弹窗可复用同一套稳定架构；
- macOS、Windows 与 Web Preview 的浮层行为会更加一致。
