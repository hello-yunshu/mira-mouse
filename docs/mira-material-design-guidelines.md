# Mira 材质层级设计方案

本文用于指导 AI 或开发者调整 Mira Mouse 的界面材质、透明度、毛玻璃、卡片、按钮和浮层。目标不是把界面改成全局毛玻璃，而是建立一套稳定的材质层级：内容要清楚，操作要可信，浮层要轻盈，Mira 要保留温柔、精致、桌面原生的气质。

## 设计结论

Mira 最适合采用：

```text
Opaque content surfaces + glass floating surfaces
稳定内容面 + 轻盈浮动玻璃
```

也就是：

- 长时间存在的信息区、设置区、读数区、按钮和输入控件应偏不透明。
- 临时出现、可轻松关闭、浮在主界面之上的内容可以使用毛玻璃。
- modal 背后的遮罩可以半透明，但 modal 内部承载大量信息时，面板本身应该更实。
- 不要 glass-on-glass。玻璃浮层里的按钮、列表项、读数块应使用实色或半实色容器，不要继续叠一层模糊玻璃。
- 毛玻璃不是背景装饰，而是一种“浮起 / 临时 / 与背景有上下文关系”的材料。

## 参考依据

### Apple Liquid Glass

Apple 在 2025 年后把 Liquid Glass 作为跨平台材质语言，强调它用于 controls、navigation、sheet、floating UI 等功能层，而不是让所有内容面都透明。其重点是：玻璃帮助交互层从内容上浮起，内容本身仍然需要保持可读性和结构。

对 Mira 的启发：

- 主内容不要全部玻璃化。
- 系统级导航、工具栏和临时浮层可以有玻璃感；内容区里的普通按钮、输入框和写入控件应更稳、更实。
- 当任务打断主流程时，使用 dimming layer 或 scrim 聚焦注意力。
- 当浮层进入更深交互状态时，应更稳定、更不透明，而不是更透明。

参考：

- https://developer.apple.com/design/human-interface-guidelines/materials
- https://developer.apple.com/documentation/technologyoverviews/adopting-liquid-glass
- https://developer.apple.com/videos/play/wwdc2025/219/
- https://developer.apple.com/videos/play/wwdc2025/356/

### Microsoft Fluent 2

Fluent 2 明确拆分了材质：

- Solid：最常用的不透明材料，用于普通 UI 区域。
- Mica：长时间存在的窗口背景，偏不透明，带轻微环境 tint。
- Acrylic：半透明毛玻璃，用于 transient / light-dismiss surfaces，比如 popover、menu、flyout。
- Smoke：modal 背后的暗化层，用于让底层界面退后。

对 Mira 的启发：

- Acrylic 不应该铺满所有卡片。
- Settings card、About card、读数块、普通按钮都应该更接近 Solid。
- battery popover、device switcher、通知、tooltip、轻量编辑浮层适合 Acrylic 风格。
- blocking modal 应该有 Smoke / scrim，并让 modal 内容面板更稳。

参考：

- https://fluent2.microsoft.design/material
- https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic
- https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/materials
- https://learn.microsoft.com/en-us/windows/apps/develop/ui/system-backdrops

### Material Design 3

Material 3 不以毛玻璃为核心，但它对 Mira 很有价值：普通界面层级通过 surface container、tonal color、elevation 建立，而不是靠透明度。按钮、卡片、容器都应有稳定的背景角色。

对 Mira 的启发：

- 把 `surface`、`surface-container`、`surface-container-high` 的思想引入 token。
- 普通卡片不要依赖 backdrop-filter 表达层级。
- 层级可以通过色调、边框、阴影、密度和间距完成。

参考：

- https://m3.material.io/styles/color/roles
- https://m3.material.io/styles/elevation
- https://m3.material.io/styles/elevation/applying-elevation
- https://m3.material.io/foundations/design-tokens

### 优秀产品案例

Raycast 的最新桌面端方向很适合 Mira 参考：它强调快速、可靠、低噪音，并把 Settings 作为清晰、集中的配置界面，而不是视觉表演场。Linear 的 UI 改版也强调减少视觉噪音、保持对齐、提高导航和面板层级密度。这两类产品都说明：工具型桌面应用要好看，核心不是到处透明，而是信息密度、层级、对齐、稳定反馈。

对 Mira 的启发：

- Mira 是设备控制工具，读数和写入操作必须像仪表一样可信。
- 可以保留柔和、可爱、发光的氛围，但控制面板本身要稳定。
- 动态、玻璃和光感应该集中在 device aura、通知、popover、tooltip 和状态反馈，而不是每个块都漂浮。

参考：

- https://www.raycast.com/
- https://manual.raycast.com/settings
- https://www.raycast.com/blog/the-new-raycast
- https://www.raycast.com/blog/a-technical-deep-dive-into-the-new-raycast
- https://linear.app/now/how-we-redesigned-the-linear-ui

注意：Raycast 和 Linear 在本文中是审美与信息架构参考，不是 Mira 要复刻的视觉系统。Mira 仍应保留自己的设备光效、柔和 accent、紧凑控制台布局和插件驱动结构。

## Mira 的目标气质

Mira 的关键词：

- 精致：像原生桌面应用，而不是网页主题皮肤。
- 温柔：色彩柔软，有一点设备灯效的灵气。
- 可信：鼠标设置、DPI、回报率、灯光写入这些操作不能显得飘。
- 轻盈：弹窗和通知可以有空气感，但文字必须清楚。
- 插件友好：宿主只定义语义层级，插件内容进入既定 surface，不让插件自己发明材质。

一句话视觉方向：

```text
Mira 是一个有柔光氛围的精密设备控制台，不是一个全透明展示橱窗。
```

## 平台材质能力与适配结论

Mira 不能把“毛玻璃”当作跨平台完全一致的能力。macOS、Windows、Linux 和 Web preview 的真实材质能力不同，设计语言应该保持一致，但实现要分平台。

### macOS

能力判断：

- macOS 可以通过 native Vibrancy / `NSVisualEffectView` 提供真实系统级毛玻璃氛围。
- Mira 当前 Tauri 路径已经在 macOS setup 中调用 `apply_vibrancy(..., NSVisualEffectMaterial::UnderWindowBackground, ...)`。
- WebView 内部 DOM 浮层再叠 `backdrop-filter` 时，可以形成接近系统层级的玻璃效果。
- Tooltip 必须继续通过 React Portal 渲染到 `body`，绕开 `.card` 的 backdrop root 截断，否则 tooltip 的 blur 采样会被父级隔离。

适配策略：

- macOS native path 可以允许更轻、更透明的浮层。
- `.platform-macos:not(.web-preview)` 可以保留较低 opacity 的 `--floating-glass-bg` 和 `--tooltip-glass-bg`。
- macOS 上 tooltip、battery popover、device switcher、轻量 edit modal 可以保留 backdrop blur。

### Windows

能力判断：

- Windows 官方可以做 Acrylic / Mica / Desktop Acrylic，但主要是窗口或 WinUI/XAML surface 的系统背景能力。
- Mira 当前 Tauri 路径通过 `window-vibrancy` 对主窗口调用 `apply_acrylic()`，失败时 fallback 到 `apply_mica()`。这能给整个窗口提供 Windows 系统背景氛围。
- Mira 的 battery popover、device switcher、notification、tooltip、edit modal 目前都是 WebView 内部 DOM 浮层，不是独立 WinUI Flyout，也不是独立 native child window。
- 因此 Windows 上这些 DOM 浮层不能可靠拥有和 macOS 一样的“系统级弹窗毛玻璃”。CSS `backdrop-filter` 最多是 WebView 内页面内容的模拟 blur，不能当作真实 Acrylic 弹窗能力。

结论：

```text
Windows 可以有 Acrylic-like 的 Mira 浮层，但不应宣称 DOM 弹窗具备真实系统毛玻璃。
```

适配策略：

- Windows native path 的浮层应使用更实的 acrylic-like / solid translucent fallback。
- `.platform-windows:not(.web-preview)` 应设置更高 opacity 的 `--floating-glass-bg`，并关闭 DOM 浮层 blur。
- Tooltip 因为通过 Portal 到 `body`，也需要单独的 Windows 分支：背景更实，`--tooltip-glass-blur: none`。
- Windows 的“玻璃感”主要来自主窗口 Acrylic/Mica 背景、柔和 tint、边框、阴影和高光，而不是每个 DOM popup 自己 blur。

如果未来必须追求 Windows 真 popup Acrylic，只有两条高成本路线：

- 把浮层改成独立 native child window，并对每个窗口应用 Acrylic/Mica；代价是焦点、定位、动画、无障碍和跨平台复杂度都会显著上升。
- 接入 WinUI/XAML 原生 Flyout/Popup；代价是 React/Tauri 组件体系会被拆开，插件 UI 也更难保持一致。

当前推荐：不要走这两条。Mira 应在 Windows 使用更稳的 Acrylic-like fallback，保持美观和可靠。

### Linux

能力判断：

- Linux 桌面环境、合成器和 WebView 后端差异很大，不能假设存在稳定 native blur。
- Linux 默认应走 solid / semi-opaque fallback。

适配策略：

- 不依赖毛玻璃作为可读性前提。
- 真实非 macOS/Windows 运行时使用 `.platform-fallback`，关闭 DOM blur，并使用更实的 surface。
- 使用 `surface`、border、shadow 和 accent 来表达层级。
- 若未来某些桌面环境支持 blur，也只能作为渐进增强，不作为基线。

### Web preview

能力判断：

- Web preview 只能验证 CSS 变量、布局、透明度和普通 `backdrop-filter` 的页面内效果。
- 它不能证明 macOS native Vibrancy 或 Windows Acrylic/Mica 的最终合成效果。

适配策略：

- preview 可以保留 CSS blur 模拟，用来快速看层级。
- 最终平台验收必须至少检查 computed styles；有条件时再看真实 macOS / Windows 应用截图。

## 材质层级

### Layer 0: Window atmosphere

用途：

- 整个窗口底层。
- macOS native vibrancy / Windows Acrylic or Mica 的平台氛围。
- device aura 背后的空间感。

设计：

- 可以透明或微透明。
- 不承载正文可读性。
- 在真实 macOS / Windows native path 中避免重复 CSS backdrop blur。
- web preview 可以保留 CSS blur 以模拟桌面效果。

建议 token：

```css
--mira-window-tint-light: rgb(255 255 255 / 4%);
--mira-window-tint-windows-light: rgb(247 245 248 / 42%);
--mira-window-tint-dark: rgb(16 14 22 / 18%);
--mira-window-tint-windows-dark: rgb(16 14 22 / 56%);
```

### Layer 1: Opaque content surface

用途：

- `.card`
- `.settings-section`
- `.about-section`
- 插件主要内容区
- DPI、回报率、灯光等长期存在的控制区

设计：

- 偏不透明或半实色，不使用强 backdrop blur。
- 阴影很轻，主要靠边框、填充、间距和标题分割建立层级。
- 这是 Mira 的“仪表盘表面”，必须稳。

建议目标范围：

```css
--surface-bg: rgb(255 255 255 / 42%); /* 起步值 */
--surface-bg-strong: rgb(255 255 255 / 64%); /* 信息密集或 modal 内容 */
--surface-border: rgb(90 82 108 / 12%);
--surface-shadow: none;

--surface-bg-dark: rgb(38 35 46 / 58%); /* 起步值 */
--surface-bg-strong-dark: rgb(38 35 46 / 74%);
--surface-border-dark: rgb(255 255 255 / 10%);
```

若需要保留 macOS 轻透感，可以不要一次改得太实，先从当前 `--panel: rgb(255 255 255 / 12%)` 提升到：

```css
--panel: rgb(255 255 255 / 34%);
```

再通过视觉检查决定是否提升到 `48%` 或 `64%`。

重要：这里的数值是分阶段起点和上限，不是要求所有卡片一次性改到 `64%`。Mira 的美感来自“底层有空气，内容面稳定”，如果所有 surface 同时过实，会从精致桌面工具变成普通后台面板。

### Layer 2: Inner grouped surface

用途：

- `.capability-summary`
- `.lighting-group`
- `.capability-group`
- 表格式读数区
- 插件内部的小组块

设计：

- 比外层 card 稍微更实，帮助用户看清每组数据。
- 不使用 backdrop-filter。
- 圆角小于外层 card。
- 不强阴影，只用轻 border / fill。

建议：

```css
--surface-group-bg: rgb(127 120 140 / 8%);
--surface-group-bg-hover: rgb(127 120 140 / 11%);
--surface-group-border: rgb(127 120 140 / 10%);
```

### Layer 3: Control surface

用途：

- 普通按钮
- secondary button
- icon button
- segmented controls
- tabs
- select
- input
- slider value chip

设计：

- 控件必须清楚可点击。
- 普通控件不要毛玻璃化。
- 内容区里会触发设备写入的控件尤其要稳，不能像装饰性浮层。
- active state 可以用 Mira accent gradient，但不要每个普通按钮都发光。
- icon button 可以透明，但 hover/active 要有明确实色反馈。

建议：

```css
--control-bg: rgb(74 70 84 / 96%);
--control-bg-hover: rgb(90 86 102 / 96%);
--control-secondary-bg: rgb(255 255 255 / 42%);
--control-secondary-border: rgb(90 82 108 / 14%);
--control-hover-bg: rgb(127 120 140 / 10%);
```

active / primary 继续使用现有 Mira 柔和渐变：

```css
linear-gradient(
  115deg,
  color-mix(in oklch, var(--accent), #8fa8e4 36%),
  color-mix(in oklch, var(--accent), #d67fb2 28%)
)
```

### Layer 4: Floating glass surface

用途：

- `.device-switcher-popover`
- `.battery-popover`
- `.app-notification`
- `.edit-modal`
- 轻量 popover / flyout / menu

设计：

- 这是 Mira 可以使用毛玻璃的主场。
- 背景要足够厚，不能让背后文字清楚可读。
- 使用更强 shadow 表示浮起。
- 内部按钮和列表项用实色 hover，不再叠毛玻璃。

当前可保留的方向：

```css
--floating-glass-bg: rgb(247 244 250 / 82%);
--floating-glass-bg-dark: rgb(35 32 43 / 78%);
--floating-glass-blur: blur(28px) saturate(175%);
--floating-glass-shadow:
  0 10px 34px rgb(47 39 60 / 13%),
  inset 0 1px 0 rgb(255 255 255 / 32%),
  0 20px 52px rgb(42 35 52 / 20%);
```

需要注意：

- macOS native path 可以更透明，因为背后是真系统 vibrancy。
- Windows path 应更实，并关闭 DOM 浮层 blur；它是 Acrylic-like fallback，不是 macOS 式真弹窗毛玻璃。
- Linux path 默认走 solid / semi-opaque fallback，不把 blur 当作基线能力。
- web preview 只能检查 CSS，不等于真实平台合成效果。

### Layer 5: Tooltip special glass

用途：

- `.tooltip-content`
- 帮助说明
- 非操作型的短文案提示

设计：

- Tooltip 是 Mira 的特例：它通过 React Portal 到 body，绕开 `.card` 的 backdrop root 截断。
- 它可以比普通 floating glass 更轻，但必须保证背后字不可清楚阅读。
- tooltip 宽度保持 `min(240px, 50vw, calc(100vw - 32px))`。

保留原则：

```text
tooltip 不跟所有 floating surface 完全共享同一个背景变量。
```

原因：

- tooltip 的视觉问题来自 CSS backdrop root 和 portal，而不是普通 popover 层级。
- 如果用全局 `--glass-popup` 调 tooltip，很容易误伤通知、battery popover、edit modal。

### Layer 6: Modal scrim / smoke

用途：

- blocking dialog
- 设备详情 modal 背景
- 未来可能的确认弹窗

设计：

- 背后用轻 smoke 让主界面退后。
- modal 面板本身根据内容密度选择材质：
  - 少量轻交互：floating glass。
  - 大量读数或复杂设置：分平台——macOS 用 floating glass（透出 vibrancy + CSS blur），Windows/Linux 用 solid-elevated surface。

建议：

```css
--scrim-bg: rgb(30 24 38 / 10%);
--scrim-bg-strong: rgb(30 24 38 / 18%);
```

## 组件映射表

| 组件 / selector | 推荐材质 | 是否 backdrop-filter | 说明 |
| --- | --- | --- | --- |
| `.app-shell` | Window atmosphere | web preview only | native path 避免重复 blur |
| `.card` | Opaque content surface | no | 主信息块不应全局玻璃 |
| `.settings-section` | Opaque content surface | no | 设置页需要稳定、清楚 |
| `.about-section` | Opaque content surface | no | 文本较多，透明会降低阅读 |
| `.control-tabs` | Control surface | preferably no | segmented control 应像控件，不像浮窗 |
| `.lighting-sub-tabs` | Control surface | preferably no | 与 control-tabs 同语言 |
| `.capability-summary` | Inner grouped surface | no | 数据容器，偏实 |
| `.lighting-group` | Inner grouped surface | no | 外层小组块 |
| `.lighting-row` | Inner data tile | no | 当前更亮的内层块方向是对的 |
| `.device-switcher-popover` | Floating glass | yes | 轻 dismiss 浮层 |
| `.battery-popover` | Floating glass | yes | 轻 dismiss 浮层 |
| `.app-notification` | Floating glass | yes | 临时状态反馈 |
| `.edit-modal` | Floating glass or solid-elevated | yes for small editor | 若内容变复杂则改实 |
| `.device-details` | macOS floating glass / Windows solid | macOS yes / Windows no | 读数密集；macOS 透出 vibrancy，Windows 实色 |
| `.tooltip-content` | Tooltip special glass | yes | 单独变量，不并入普通浮层 |
| `.icon-button` | Control surface | no | 默认透明可行，hover 要实 |
| `.secondary` | Control surface | no | 不建议毛玻璃 |

## 建议的 token 重构

当前 `src/styles.css` 主要 token 是 `--panel`、`--glass-popup`、`--tooltip-glass-bg`、`--floating-glass-*`。建议逐步增加语义 token，先不大规模重写：

```css
:root {
  --surface-bg: rgb(255 255 255 / 42%);
  --surface-bg-strong: rgb(255 255 255 / 64%);
  --surface-border: rgb(90 82 108 / 12%);
  --surface-group-bg: rgb(127 120 140 / 8%);
  --surface-group-border: rgb(127 120 140 / 10%);

  --control-secondary-bg: rgb(255 255 255 / 42%);
  --control-hover-bg: rgb(127 120 140 / 10%);

  --floating-glass-bg: rgb(247 244 250 / 82%);
  --floating-glass-border: rgb(255 255 255 / 34%);
  --floating-glass-blur: blur(28px) saturate(175%);

  --tooltip-glass-bg: rgb(247 244 250 / 72%);
  --scrim-bg: rgb(30 24 38 / 10%);
}

:root[data-theme="dark"] {
  --surface-bg: rgb(38 35 46 / 58%);
  --surface-bg-strong: rgb(38 35 46 / 74%);
  --surface-border: rgb(255 255 255 / 10%);
  --surface-group-bg: rgb(255 255 255 / 7%);
  --surface-group-border: rgb(255 255 255 / 9%);

  --control-secondary-bg: rgb(255 255 255 / 9%);
  --control-hover-bg: rgb(255 255 255 / 10%);

  --floating-glass-bg: rgb(35 32 43 / 78%);
  --floating-glass-border: rgb(255 255 255 / 10%);

  --tooltip-glass-bg: rgb(35 32 43 / 78%);
  --scrim-bg: rgb(0 0 0 / 18%);
}
```

命名原则：

- `surface-*`：长期内容。
- `control-*`：按钮、tabs、input 等可操作控件。
- `floating-glass-*`：临时浮层。
- `tooltip-*`：tooltip 特例。
- `scrim-*`：遮罩。

执行原则：

- 这些 token 是试调起点，不是最终硬编码答案。
- 先在 light / dark / system 三种主题里检查整体气质，再决定是否继续加实。
- 如果某个页面开始显得笨重，优先降低外层 card 的 opacity，而不是削弱浮层的玻璃感。

## 实施步骤

### Step 1: 先改语义 token，不改布局

在 `src/styles.css` 中增加新的 surface/control/scrim token，并让旧 token 暂时指向新 token：

```css
--panel: var(--surface-bg);
--panel-border: var(--surface-border);
```

不要第一步就删除旧变量，避免影响范围不可控。

### Step 2: 让普通 card 退出毛玻璃

目标 selector：

```css
.card
.settings-section
.about-section
```

建议：

- `background: var(--surface-bg);`
- 删除或关闭 `backdrop-filter`。
- 保持 `box-shadow: none;`
- 保留边框。

验收：

- Settings 页面不再像一层一层玻璃叠起来。
- 卡片里的文字清晰。
- 背景氛围仍存在，但不会穿透到正文。

### Step 3: 统一 grouped/data surfaces

目标 selector：

```css
.capability-summary
.capability-summary span
.lighting-group
.lighting-row
.capability-group
```

建议：

- 外层 group 用 `--surface-group-bg`。
- 内层 row/tile 比 group 更亮一点。
- 不加 backdrop-filter。
- 保持轻 border 和轻 inset highlight。

验收：

- 灯光和回报率等读数区看起来是一套。
- 内部块不漂，不抢过主操作。

### Step 4: 控件改成稳定实体

目标 selector：

```css
.secondary
.icon-button
.control-tabs
.lighting-sub-tabs
select
input
```

建议：

- 默认状态用 control surface。
- hover 使用 `--control-hover-bg`。
- active/primary 才使用 accent gradient。
- tabs 容器不再需要 blur，active pill 才突出。

验收：

- 用户一眼知道哪里可点击。
- tabs 不像 floating panel。
- 选中态明确但不刺眼。

### Step 5: 保留并强化 floating glass

目标 selector：

```css
.device-switcher-popover
.battery-popover
.app-notification
.edit-modal
body:has(.platform-windows:not(.web-preview)) .tooltip-content
```

建议：

- macOS 继续使用 `--floating-glass-*` 的 blur。
- Windows 使用更实的 `--floating-glass-bg`，并让 `--floating-glass-blur: none`。
- 保持强 shadow。
- light mode 背景至少 `72%`，当前 `82%` 是安全值。
- dark mode 背景至少 `72%`。
- 内部 hover 不用 blur，只用半实色。

验收：

- 浮层和主内容明显分离。
- 浮层背后的字不能清楚读出来。
- notification 仍然轻盈，不像系统错误框。

### Step 6: 保留 tooltip 特例

目标 selector：

```css
.tooltip-content
body:has(.platform-macos:not(.web-preview)) --tooltip-glass-bg
```

建议：

- 不要把 tooltip 背景直接改成 `--floating-glass-bg`。
- 保持 portal 注释和实现。
- 调 tooltip 透明度时只改 `--tooltip-glass-bg`。

验收：

- 设置页问号 tooltip 仍有毛玻璃感。
- 背后文字不可清楚读。
- 调 tooltip 不影响 battery popover / notification / edit modal。

### Step 7: modal 视内容密度分级

目标 selector：

```css
.details-backdrop
.device-details
.edit-modal-backdrop
.edit-modal
```

建议：

- `.edit-modal`：小型编辑器，可以保留 floating glass。
- `.device-details`：信息密集。macOS 用 floating glass（`--floating-glass-bg` + backdrop-filter），透出窗口 vibrancy；Windows/Linux 用 solid-elevated（`--surface-bg-strong` + 无 blur），保证读数可读性。
- backdrop 使用 `--scrim-bg`，不要完全透明。

验收：

- 打开设备详情时，背后 dashboard 退后。
- macOS 上详情面板有毛玻璃感（透出 vibrancy + CSS blur）；Windows/Linux 上是稳定实色面板。

### Step 8: reduced transparency

`@media (prefers-reduced-transparency: reduce)` 必须继续：

- 关闭所有 backdrop-filter。
- floating glass 改成 solid fallback。
- tooltip 改成 solid fallback。
- 不降低文字对比度。

## AI 修改提示词

可以直接把下面这段交给 AI 执行：

```text
请按 docs/mira-material-design-guidelines.md 调整 Mira 的材质层级。核心目标是：

1. 不要全局毛玻璃。
2. 主内容卡片、设置块、读数块、按钮和 tabs 改为更稳定的 opaque / semi-opaque surface。
3. 只让临时浮层继续使用 floating glass：device switcher、battery popover、app notification、小型 edit modal。
4. tooltip 是特殊毛玻璃，不要和普通 floating glass 合并变量。
5. device details 信息密集，应该比普通浮层更实；backdrop 用轻 scrim。
6. 保留 Mira 当前柔和 accent、device aura、圆角和紧凑布局，不做大改版。
7. 先新增语义 token，再逐步替换 selector，避免一次性破坏已有平台差异。
8. 每步完成后检查 light/dark/system、macOS native path、Windows native path、web preview、prefers-reduced-transparency。
9. 文档中的 opacity 数值是起步范围：先小步试调，再通过截图和 computed styles 验证，不要机械套用到所有组件。
10. Windows native DOM 浮层不要依赖 `backdrop-filter`；macOS 可以保留真玻璃，Windows 和 Linux 必须有 solid / acrylic-like fallback。
11. 非 macOS/Windows 的真实运行时应使用 `.platform-fallback`，不要落回 web preview 的默认 blur。

重点文件：

- src/styles.css
- src/App.tsx
- src/Settings.tsx
- src/About.tsx

不要改插件契约、运行时逻辑、i18n 文案或设备写入行为，除非视觉修改必须触及对应组件结构。
```

## 验收清单

视觉验收：

- Settings 页面：卡片稳定，不像多层透明玻璃。
- Dashboard：device aura 仍然有氛围，但控制区清楚。
- DPI / 回报率 / 灯光读数：内部块统一、居中、可读。
- Button / tabs：默认实体，active 有 Mira accent。
- Battery popover：浮起、轻盈、可读。
- Device switcher：像临时菜单，不像普通卡片。
- Notification：右上角玻璃状态区保留。
- Tooltip：问号帮助有玻璃感，但背后字不可读。
- Device details：大量信息不被背景干扰。

平台验收：

- macOS：native vibrancy 路径不被重复 CSS blur 污染。
- Windows：Acrylic/Mica fallback 下卡片不会显脏，浮层更实。
- Windows：DOM popover / tooltip 不依赖 `backdrop-filter` 才能可读；没有 blur 时仍然像 Mira。
- macOS：popover / tooltip 可以保留更轻的玻璃，但背后文字不可清楚读。
- Linux：没有 native blur 时也应是完整可用的 solid surface。
- Web preview：能大致模拟层级，但不要把它当作 native 合成最终效果。
- Dark mode：浮层不透脏，普通卡片不灰糊。
- Reduced transparency：所有 blur 关闭后仍然好看、清楚。

技术验收：

- CSS token 语义清晰。
- 不引入 brand/model specific host hardcoding。
- 不改变 plugin writable contract。
- 不改变 device snapshot / mutation behavior。
- 不让 tooltip 修改误伤其他 floating surface。
- 不让 card 内部的 backdrop root 再次破坏 tooltip。

## 不做的事

- 不把 Mira 改成 Apple 官方控件复刻。
- 不把所有区域改成完全不透明的企业后台。
- 不引入大面积紫蓝渐变背景。
- 不新增装饰性 orb / blob。
- 不把设备灯效的颜色逻辑改成 UI 主题色逻辑。
- 不把插件内容改成品牌专属布局。

## 最终判断标准

如果一个区域是用户要长期阅读、比较、点击、配置的地方，它应该更实。

如果一个区域是临时出现、依附某个触发点、可轻松关闭、用于提示或小范围操作的地方，它可以是毛玻璃。

如果一个区域承载关键写入或复杂读数，它宁可少一点梦幻，也要更清楚可信。
