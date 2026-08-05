# rill-ml 1.1.0 电量回放对比报告

> 由 `cargo run -p mira-local-ai --bin replay_report -- report` 生成，读取已提交的 `tests/fixtures/replay/*.json`，对每个场景在 plain / weighted / robust 三种配置下运行 `predict`。

## 对比表

| 场景 | 配置 | source | 训练样本 | 校验样本 | baseline MAE | candidate MAE | weighted MAE | recent MAE | 剩余(h) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| battery_aging | plain | Baseline | 30 | 24 | 1.2628 | 2.6093 | 2.6093 | 2.7886 | — |
| battery_aging | weighted | Baseline | 30 | 24 | 1.2628 | 4.1549 | 4.1900 | 4.1742 | — |
| battery_aging | robust | Baseline | 30 | 24 | 1.2628 | 2.7112 | 2.7112 | 2.9867 | — |
| battery_replacement | plain | LocalAI | 24 | 18 | 2.0326 | 1.8909 | 1.8909 | 0.4509 | 32.20 |
| battery_replacement | weighted | Baseline | 24 | 18 | 2.0326 | 2.9440 | 2.3125 | 1.0586 | — |
| battery_replacement | robust | LocalAI | 24 | 18 | 2.0326 | 1.8909 | 1.8909 | 0.4509 | 32.20 |
| dpi_switch | plain | LocalAI | 33 | 27 | 3.0055 | 2.4322 | 2.8432 | 0.3725 | 46.37 |
| dpi_switch | weighted | LocalAI | 33 | 27 | 3.0055 | 3.3648 | 2.5032 | 0.4793 | 57.31 |
| dpi_switch | robust | LocalAI | 33 | 27 | 3.0055 | 2.4322 | 2.8432 | 0.3725 | 46.37 |
| invalid_zero_reading | plain | Baseline | 20 | 14 | 0.0000 | 1.9325 | 1.9325 | 1.6321 | — |
| invalid_zero_reading | weighted | Baseline | 20 | 14 | 0.0000 | 3.1742 | 3.0460 | 2.7475 | — |
| invalid_zero_reading | robust | Baseline | 20 | 14 | 0.0000 | 1.9325 | 1.9325 | 1.6321 | — |
| lighting_switch | plain | LocalAI | 33 | 27 | 3.7568 | 2.5260 | 3.0182 | 0.4074 | 47.26 |
| lighting_switch | weighted | LocalAI | 33 | 27 | 3.7568 | 3.9721 | 2.9364 | 0.4358 | 56.47 |
| lighting_switch | robust | LocalAI | 33 | 27 | 3.7568 | 2.5260 | 3.0182 | 0.4074 | 47.26 |
| missing_context | plain | Baseline | 30 | 24 | 0.0000 | 1.7147 | 1.7147 | 1.3982 | — |
| missing_context | weighted | Baseline | 30 | 24 | 0.0000 | 3.3412 | 3.0644 | 2.5063 | — |
| missing_context | robust | Baseline | 30 | 24 | 0.0000 | 1.7147 | 1.7147 | 1.3982 | — |
| mixed_long_history | plain | Baseline | 40 | 34 | 1.8430 | 2.2826 | 2.4668 | 2.4442 | — |
| mixed_long_history | weighted | Baseline | 40 | 34 | 1.8430 | 3.4799 | 3.5063 | 3.2841 | — |
| mixed_long_history | robust | Baseline | 40 | 34 | 1.8430 | 2.2826 | 2.4668 | 2.4442 | — |
| polling_switch | plain | LocalAI | 33 | 27 | 2.6298 | 2.0044 | 2.3504 | 0.3493 | 47.76 |
| polling_switch | weighted | LocalAI | 33 | 27 | 2.6298 | 3.0111 | 2.2654 | 0.5100 | 57.81 |
| polling_switch | robust | LocalAI | 33 | 27 | 2.6298 | 2.0044 | 2.3504 | 0.3493 | 47.76 |
| reconnect_jump | plain | Baseline | 20 | 14 | 0.0000 | 1.9278 | 1.9278 | 1.6240 | — |
| reconnect_jump | weighted | Baseline | 20 | 14 | 0.0000 | 3.1698 | 3.0409 | 2.7412 | — |
| reconnect_jump | robust | Baseline | 20 | 14 | 0.0000 | 1.9278 | 1.9278 | 1.6240 | — |
| steady_usage | plain | Baseline | 30 | 24 | 0.0000 | 1.7119 | 1.7119 | 1.3953 | — |
| steady_usage | weighted | Baseline | 30 | 24 | 0.0000 | 3.3392 | 3.0621 | 2.5034 | — |
| steady_usage | robust | Baseline | 30 | 24 | 0.0000 | 1.7119 | 1.7119 | 1.3953 | — |

## 场景解读

> 质量门 `candidate MAE < baseline MAE × 0.98` 是较高的门槛，回退到 Baseline 也是正确结果。下表结论基于 `recent MAE`（最近 24 小时验证窗口）的 plain vs weighted 相对趋势，不把“未通过质量门”误判为失败。

| 场景 | 结论 |
|---|---|
| battery_aging | **加权退化** | recent MAE 4.1742 vs 2.7886（+50%）。期望：模型应能跟踪缓慢漂移；加权模型不因旧阶段拖累近期预测。 |
| battery_replacement | **加权退化** | recent MAE 1.0586 vs 0.4509（+135%）。期望：更换电池应被识别为放电段边界，不让跳升成为错误高耗电标签；预测不崩溃。 |
| dpi_switch | **加权退化** | recent MAE 0.4793 vs 0.3725（+29%）。期望：加权模型应比普通模型更快适应近期低 DPI 状态（recent MAE 更低）。 |
| invalid_zero_reading | **加权退化** | recent MAE 2.7475 vs 1.6321（+68%）。期望：0% 读数不应破坏历史或崩溃；后续预测仍可用。 |
| lighting_switch | **加权退化** | recent MAE 0.4358 vs 0.4074（+7%）。期望：加权模型应比普通模型更快适应近期无灯光低耗电状态。 |
| missing_context | **加权退化** | recent MAE 2.5063 vs 1.3982（+79%）。期望：缺少上下文不应阻塞预测；上下文特征按 0 处理，仍产出合法结果。 |
| mixed_long_history | **加权退化** | recent MAE 3.2841 vs 2.4442（+34%）。期望：长历史 + 上下文混合下，普通与加权模型都应稳定、不崩溃。 |
| polling_switch | **加权退化** | recent MAE 0.5100 vs 0.3493（+46%）。期望：加权模型应比普通模型更快适应近期低回报率状态。 |
| reconnect_jump | **加权退化** | recent MAE 2.7412 vs 1.6240（+69%）。期望：突升应被识别为边界，不产生错误标签；加权模型对异常样本稳健。 |
| steady_usage | **加权退化** | recent MAE 2.5034 vs 1.3953（+79%）。期望：普通与加权模型输出应接近；加权 MAE ≈ 普通 MAE，均保持稳定。 |

### 汇总

在 10 个场景中，加权模型相对普通模型的近期误差：**提升 0 个、退化 10 个、相当 0 个**。

- 本组固定 fixture（recency tau = Some(48.0)h）下，加权模型未展现出对普通模型的系统性近期优势，部分场景略退化。
- 因此 `weighted_learning_enabled` 保持**默认关闭**符合阶段 3 验收标准（“默认加权学习不降低历史回放总体稳定性”；未满足“适应速度明显快于旧模型”时不擅自默认启用）。
- 该结论仅针对当前 fixture 与 tau 取值；调整 `learning_recency_tau_hours` 或补充真实遥测回放后需重新评估，不能据此推断加权学习在真实数据上无效。

## 说明与限制

- `max_error` / `convergence_time` 未由 `BatteryPredictionOutput` 暴露，本报告以 `recent MAE` 作为近期适应度代理。
- 所有配置默认关闭（`weighted_learning_enabled` / `robust_detection_enabled` 均为 false），开启仅用于对比，不改变生产默认行为。
- 本轮不验证 IPC V3 性能（阶段 5 仅骨架、默认关闭）；IPC 字节/时延对比留待状态化路径转正前的专项测试。
