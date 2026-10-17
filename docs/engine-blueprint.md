# Blueprint 引擎 v2（core/app 接口）

本文取代旧文档中 Void 排序、required 拓扑、literal 设定值和运行结束后 writes_param 的描述。

## 存档和迁移

`Workflow::new` 创建 `version: 2`。缺少 version 的 JSON 按 v1 读取。`resolve/run` 对非 v2 显式报错，不执行任何节点。app 当前拒绝加载非 v2，保留原存档；尚无迁移 UI，不提供自动推测执行顺序的迁移。

显式迁移需要确认：显式入口及 Exec 连线、Sequence 分支顺序、Loop body/completed 路径；
旧 `param:value` 设定值连线需要重新接到对应变量节点的 Set 口。不能仅把 version 改成 2。
迁移失败应保留原存档。工作流没有全局变量表；变量是节点本地状态。

## core/app 接口

- **端口分类**：使用 `PortDef.exec: bool`，默认 false；`PortDef::exec(id,label)` 创建执行引脚。Exec 不携带 Value，不能接到 Data（即使 Data 是 Any/Void）。为兼容旧类型显示，Exec 的 ty 保留 Void 占位，但绝不能再按 ty/required 推断执行。连线前比较 exec，再对 Data 校验类型。
- **端口元数据**：使用 `engine::resolve` 的实例端口，或 `NodeSpec::inputs_for/outputs_for`。`Registry::kinds` 已提供默认解析端口及 `pure` 元数据。不要直接使用未经解析的 kind.inputs。
- **普通副作用节点**：默认自动获得 `exec` 输入、`then` 输出；入口无 exec 输入。读取文件、图像处理、保存等保守地保留为 Exec 节点。连接数据不再自动执行来源节点。
- **入口**：内置 start 的 `out` 是 Exec。每图最多一个入口；没有入口的副作用不会运行（警告），不猜测多个入口的先后。
- **变量节点**：数字 / 文本 / 布尔通过 `VariablePorts { value_param, get, set }` 注册。没有 Exec；Get 是普通数据输出，Set 是可多连的数据输入。上游节点产出值时写入目标变量状态，后续 Get 读到新值。Set 不参与 Get 的求值依赖，不会因此形成数据环。
- **Sequence**：输入 `exec`；`out1..outN` 全是 Exec。按数字序号深度优先执行整个分支后才下一分支。每个 Exec 输出只允许一条边，扇出请增加 Sequence 分支。多个 Exec 输入来源允许，分别触发分别运行。
- **Loop**：输入 `exec` 与 Data `list`；Exec `body`、Data `item`、Exec `finally`。`finally` 是 completed 的兼容端口名，仅成功完成后触发，**不是异常清理 finally**。`max=0` 表示遍历全部。空列表也触发 completed。不要把 item 当成执行引脚。
- **报告**：`RunReport.order` 是实际调用轨迹（含重复触发、按需纯求值），不是节点集合。`nodes` 也可能重复同一个 node_id；UI 按事件更新，必要时汇总。未触发节点不产生运行记录。失败停止该控制链，不把未触发节点伪装成已执行。
- **变量状态**：每次运行开始时从节点参数铺初始值。Set 写入保存在本次运行的引擎状态中，引擎不修改 Workflow。成功报告的 `variable_states`（node_id -> 标量值）由 app 写回对应节点的值参数；失败报告不提交。这样下一次运行从上次结果继续，同时变量仍随工作流存档，不引入全局变量表。
- **单步**：`run/run_with` 签名不变；`only_node` 仅允许纯节点及其纯数据依赖。副作用目标返回诊断，不能绕过 Exec。

## 扩展注册契约

`NodeSpec::fixed/dynamic` 默认 `pure=false`，安全地要求 Exec。只有无可观察副作用的回调才使用 `.pure()`，它还关闭自动产物写盘。纯节点不能声明 Exec、entry 或控制流。

`VariablePorts { value_param, get, set }` 通过 `.variable(...)` 注册，不硬编码内置 kind。
注册时声明的 Get/Set 必须是不带 Exec 的数据端口，类型一致，`value_param` 必须存在。
变量节点通常配 `.pure()`；引擎用参数初始化状态，Get 返回当前状态，Set 由数据写入。
Set 端口的 `PortDef.multi` 必须为真，才允许一个输入接多条线。

`ControlFlow::Sequence { output_prefix }` 与 `ControlFlow::Loop { list_input, item_output, body_output, finally_output, max_param }` 描述自定义端口协议。引擎仅消费协议，不识别具体 kind。普通节点最多一个 Exec 输出；多个输出必须声明控制协议。端口与控制声明不一致在 resolve 时诊断。

## 运行语义和限制

纯节点每次消费重新计算，无跨消费者缓存；变量 Get 读取的是当前运行状态，因此后续读取能看到已写入的新值。已连接的 Data 缺失或类型错误必须失败，不回退到本地参数值；未连接可选参数才使用默认/本地值。

Loop 每轮保存并恢复父运行帧；变量状态不随帧回滚。嵌套循环能读外层 item，循环内数据产物离开该帧后不可消费。数据边从不扩张执行作用域。作用域逃逸目前在运行时诊断，而非完整静态支配分析；跨循环传值应使用变量节点。

执行完全同步串行；Sequence 本身固定按分支顺序阻塞执行，没有行内开关或非阻塞模式。失败停止后续分支/轮次，不继续产生副作用。Exec 活跃路径环、深度超过 256、触发数超过 100000 会失败；纯数据环及过深依赖也失败。回调自身的阻塞/超时仍由回调负责。没有异步取消、事务回滚或自动重试。

图像/视频产物、交互和 Progress API 保留。当前 app 已接入 Exec/Data 连线校验、解析端口同步、变量节点多连输入和纯节点调试。

## 验证位置

`core/src/engine/blueprint_tests.rs`：变量 Set 写入后后续分支可见、Get/Set 多连、无纯数据环、未触发副作用、完整序列分支、循环累计/嵌套/作用域逃逸、错连和旧版本诊断。

`core/src/engine/tests.rs` 保留图像处理、产物和交互回归；旧管线夹具现在显式添加 Exec 边。视频端到端夹具同样接入显式入口。
