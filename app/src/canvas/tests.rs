//! 画布存档、端口、交互与排版的回归测试。
use super::*;
use crate::canvas::geometry::curve_maybe_cut;
use crate::canvas::layout::*;
use crate::canvas::node::*;
use crate::ui::controls::{color_rgba, color_string, hsv_to_rgb, rgb_to_hsv};
use starrytools_core::model::node_kind::ParamSpec;
use starrytools_core::model::params::Params;
use starrytools_core::model::workflow::Workflow;

#[test]
fn stale_connection_is_cleared_on_parameter_and_resolved_port_rebuilds() {
    let mut graph = canvas(&["sequence", "sequence"]);
    let param = crate::catalog::all()
        .into_iter()
        .find(|info| info.kind.id == "sequence")
        .unwrap()
        .kind
        .params
        .remove(0);
    graph.handle_control(
        0,
        &param,
        Some(crate::ui::controls::ControlEvent::Changed(
            serde_json::json!(3),
        )),
    );
    graph.touch_content();
    resolve_ports(&mut graph);
    let from = PortRef {
        node: 0,
        port: 2,
        input: false,
    };
    let to = PortRef {
        node: 1,
        port: 0,
        input: true,
    };
    assert!(graph.can_link(from, to));
    graph.interaction.connect = Some(Connect {
        from,
        to: Pos2::ZERO,
        target: Some(to),
    });
    graph.interaction.hovered_port = Some(from);
    graph.handle_control(
        0,
        &param,
        Some(crate::ui::controls::ControlEvent::Changed(
            serde_json::json!(1),
        )),
    );
    assert!(graph.interaction.connect.is_none());
    assert!(graph.interaction.hovered_port.is_none());
    let revision = graph.revision;
    graph.link(from, to);
    assert!(graph.wires.is_empty());
    assert_eq!(graph.revision, revision);
    assert_eq!(graph.check_revision(), revision);

    graph.nodes[0]
        .params
        .insert("outs".into(), serde_json::json!(3));
    graph.touch_content();
    graph.interaction.connect = Some(Connect {
        from,
        to: Pos2::ZERO,
        target: Some(to),
    });
    graph.interaction.hovered_port = Some(from);
    let revision = graph.revision;
    resolve_ports(&mut graph);
    assert!(graph.interaction.connect.is_none());
    assert!(graph.interaction.hovered_port.is_none());
    assert_eq!(graph.revision, revision);
}

#[test]
fn link_revalidates_stale_nodes_types_and_occupied_inputs_before_touching_revision() {
    let mut graph = canvas(&["sequence", "sequence", "iterate"]);
    resolve_ports(&mut graph);
    let from = PortRef {
        node: 0,
        port: 0,
        input: false,
    };
    let to = PortRef {
        node: 1,
        port: 0,
        input: true,
    };
    for stale in [
        PortRef {
            node: usize::MAX,
            ..from
        },
        PortRef {
            port: usize::MAX,
            ..from
        },
    ] {
        assert!(!graph.can_link(stale, to));
        graph.link(stale, to);
        graph.link(
            from,
            PortRef {
                input: true,
                ..stale
            },
        );
    }
    graph.link(
        from,
        PortRef {
            node: 2,
            port: 1,
            ..to
        },
    );
    assert!(graph.wires.is_empty());
    assert_eq!(graph.revision, 0);
    graph.link(from, to);
    assert_eq!(graph.wires.len(), 1);
    let revision = graph.revision;
    graph.link(from, to);
    assert_eq!(graph.wires.len(), 1);
    assert_eq!(graph.revision, revision);
}

#[test]
fn resolved_ports_ignore_old_and_duplicate_revisions_without_dirtying_document() {
    let mut graph = canvas(&["convert_image"]);
    let old = starrytools_core::engine::resolve(&graph.to_workflow(&Workflow::new("旧端口")));
    graph.touch_content();
    let before = serde_json::to_value(&graph.nodes[0].outputs).unwrap();
    graph.sync_resolved_ports(0, &old);
    assert_eq!(
        serde_json::to_value(&graph.nodes[0].outputs).unwrap(),
        before
    );
    assert_eq!(graph.ports_revision, None);
    resolve_ports(&mut graph);
    assert_eq!(graph.ports_revision, Some(1));
    assert_eq!(graph.revision, 1);
    let mut duplicate = old.clone();
    duplicate.nodes[0].outputs.clear();
    graph.sync_resolved_ports(1, &duplicate);
    assert!(!graph.nodes[0].outputs.is_empty());
    assert_eq!(graph.check_revision(), 1);
}

#[test]
fn dynamic_read_ports_use_a_placeholder_until_the_check_snapshot_arrives() {
    let mut graph = canvas(&["read"]);
    graph.nodes[0]
        .params
        .insert("valueType".into(), serde_json::json!("text"));
    refresh_ports(&mut graph.nodes[0]);
    // The resolver would immediately return TXT; UI must not invoke it here.
    assert!(graph.nodes[0].outputs.iter().any(|port| !port.exec));
    resolve_ports(&mut graph);
    assert_eq!(graph.nodes[0].outputs[0].badge(), "TXT");
    assert_eq!(graph.revision, 0);
}

#[test]
fn tool_completion_refresh_only_updates_tool_flags_once_per_revision() {
    let mut graph = canvas(&["crop_image"]);
    graph.nodes[0].requirements.model_missing = true;
    graph.nodes[0].requirements.tool_checking = true;
    let before = serde_json::to_value(&graph.nodes[0].outputs).unwrap();
    graph.sync_tool_requirements();
    assert!(!graph.nodes[0].requirements.tool_checking);
    assert!(graph.nodes[0].requirements.model_missing);
    assert_eq!(
        serde_json::to_value(&graph.nodes[0].outputs).unwrap(),
        before
    );
    // Other parallel tests may finish a probe; unchanged revision must not refresh.
    let revision = graph.tool_revision;
    graph.nodes[0].requirements.tool_checking = true;
    graph.sync_tool_requirements();
    assert_eq!(
        graph.nodes[0].requirements.tool_checking,
        graph.tool_revision == revision
    );
    assert_eq!(graph.revision, 0);
    assert_eq!(graph.check_revision(), 0);
}

/// 汉字说明不能按「字符数 ÷ 固定值」估 —— 那样一条 20 多字的说明会被估成一行，
/// 卡片就不够高、把说明截出下边框（“图像压缩”的“元数据”就是这么溢出的）。
#[test]
fn a_cjk_note_is_estimated_as_multiple_lines() {
    let note = "ICC 这类影响颜色显示的默认留着，不当垃圾删。";
    assert!(
        estimate_wrapped_height(note, NODE_W - 20.0) >= 26.0,
        "汉字说明该被估成两行以上，实际 {}（{:?}）",
        estimate_wrapped_height(note, NODE_W - 20.0),
        note,
    );
    // 短的半角说明仍是一行。
    assert!(estimate_wrapped_height("越大越慢越小。", NODE_W - 20.0) < 26.0);
}

#[test]
fn multiline_text_grows_with_content_then_scrolls_at_the_cap() {
    let short = "hello";
    let taller = "hello\nworld\nagain\nand\nagain";
    assert!(multiline_rows(taller) > multiline_rows(short));
    assert!(multiline_height(taller) > multiline_height(short));
    assert!(!multiline_scrolls(taller));

    // 没有空格的超长串也必须按字符折行，并在超过上限后改为滚动。
    let long = "verylongunbrokenstring".repeat(400);
    let longer = "verylongunbrokenstring".repeat(800);
    assert!(multiline_rows(&long) > multiline_rows(taller));
    assert!(multiline_scrolls(&long));
    assert_eq!(multiline_rows(&long), multiline_rows(&longer));
    assert_eq!(
        multiline_height(&long),
        multiline_height(&longer),
        "到达上限后高度不再增长"
    );
}

/// 换目标格式之后，节点的输出端口必须跟着变。
///
/// 这条是实际踩过的坑：端口原本是在建节点时从 catalog 抄一份，之后再也
/// 不管了 —— 于是「图像格式转换」选成 JPEG，卡片上还写着 PNG。
#[test]
fn changing_a_param_refreshes_the_ports() {
    let kinds = crate::catalog::all();
    let convert = kinds
        .iter()
        .find(|kind| kind.kind.id == "convert_image")
        .expect("应当有「图像格式转换」");

    // 参数 id 和选项值都从元数据里读，不硬编码 —— 那边改了这里不用跟着改。
    let (param_id, jpeg) = convert
        .kind
        .params
        .iter()
        .find_map(|param| match &param.spec {
            ParamSpec::Select { options, .. } => options
                .iter()
                .find(|option| option.value == "jpeg")
                .map(|option| (param.id.clone(), option.value.clone())),
            _ => None,
        })
        .expect("「图像格式转换」应当有一个能选到 jpeg 的下拉");

    let mut graph = canvas(&[&convert.kind.id]);
    resolve_ports(&mut graph);
    let before = graph.nodes[0].outputs[0].badge().to_owned();

    graph.nodes[0]
        .params
        .insert(param_id, serde_json::json!(jpeg));
    refresh_ports(&mut graph.nodes[0]);
    graph.touch_content();
    resolve_ports(&mut graph);
    let after = graph.nodes[0].outputs[0].badge().to_owned();

    assert_ne!(
        before, after,
        "换了目标格式，输出端口的类型应当跟着变（换前 {before}、换后 {after}）"
    );
}

/// 不认识的类型（老存档里的未知节点）不该把界面弄崩。
#[test]
fn an_unknown_kind_has_no_ports() {
    let (inputs, outputs) = crate::catalog::ports_for("没有这个节点", &Params::new());
    assert!(inputs.is_empty());
    assert!(outputs.is_empty());
}

/// 画布 → 工作流 → 画布：节点、位置、参数、连线都得原样回来。
#[test]
fn a_graph_survives_a_round_trip() {
    let kinds = crate::catalog::all();
    let graph = Graph::demo(12, &kinds);
    let workflow = graph.to_workflow(&Workflow::new("往返"));
    let back = Graph::from_workflow(&workflow, &kinds);

    assert_eq!(back.nodes.len(), graph.nodes.len());
    assert_eq!(back.wires.len(), graph.wires.len());

    for (before, after) in graph.nodes.iter().zip(&back.nodes) {
        assert_eq!(before.id, after.id, "节点 id 要原样保留");
        assert_eq!(before.kind, after.kind);
        assert!((before.pos - after.pos).length() < 0.001, "位置要原样保留");
        assert_eq!(before.params, after.params, "参数要原样保留");
        assert_eq!(before.outputs.len(), after.outputs.len(), "端口要重建出来");
    }

    // 连线的端口 id 也要对得上 —— 对不上就会接到别的端口上去。
    for (before, after) in graph.wires.iter().zip(&back.wires) {
        assert_eq!(before.from_port, after.from_port);
        assert_eq!(before.to_port, after.to_port);
        assert_eq!(graph.nodes[before.from].id, back.nodes[after.from].id);
        assert_eq!(graph.nodes[before.to].id, back.nodes[after.to].id);
    }

    // 刚打开的图不算「有未保存的改动」。
    assert_eq!(back.revision, 0);
}

/// 改一下画布，版本号要动 —— 未保存的小蓝点就靠它。
#[test]
fn editing_bumps_the_revision() {
    let kinds = crate::catalog::all();
    let mut graph = Graph::demo(3, &kinds);
    assert_eq!(graph.revision, 0);
    graph.touch();
    assert_eq!(graph.revision, 1);
}

/// 拖动节点只动几何：`touch` 不阭让静态检查失效，否则每帧都会重跑检查、读一次文件头。
#[test]
fn moving_a_node_does_not_invalidate_the_static_check() {
    let kinds = crate::catalog::all();
    let mut graph = Graph::demo(3, &kinds);
    let content = graph.check_revision();
    graph.touch(); // 拖动节点走的就是它
    assert_eq!(graph.revision, 1);
    assert_eq!(graph.check_revision(), content, "几何变化不该让检查失效");
}

/// 删节点是语义变化：静态检查该失效。
#[test]
fn deleting_a_node_invalidates_the_static_check() {
    let kinds = crate::catalog::all();
    let mut graph = Graph::demo(3, &kinds);
    let before = graph.check_revision();
    graph.delete_node(0);
    assert!(graph.check_revision() > before, "删节点该让检查失效");
    assert!(graph.revision > 0);
}

/// 拖拽中把那个节点删掉：不能留下越界的 `grabbed` 下标（否则下一帧 panic）。
#[test]
fn deleting_the_grabbed_node_releases_the_grab() {
    let kinds = crate::catalog::all();
    let mut graph = Graph::demo(2, &kinds);
    let last = graph.nodes.len() - 1;
    graph.interaction.grabbed = Some(last);

    graph.delete_node(last);

    assert_eq!(graph.interaction.grabbed, None, "抓着的节点没了就该松手");
    assert_eq!(graph.nodes.len(), 1);
}

/// 连线的稳定 id 在创建时就该算好，且和落盘用的拼法一致。
#[test]
fn a_wires_id_is_stable_and_matches_its_ports() {
    let mut graph = canvas(&["read", "convert_image"]);
    graph.link(
        PortRef {
            node: 0,
            port: 1,
            input: false,
        },
        PortRef {
            node: 1,
            port: 0,
            input: true,
        },
    );
    let wire = &graph.wires[0];
    assert_eq!(
        wire.id,
        Wire::make_id(
            &graph.nodes[0].id,
            &wire.from_port,
            &graph.nodes[1].id,
            &wire.to_port
        )
    );
}

/// 刀光预筛：离得远的连线不该被判定为「可能切到」。
#[test]
fn the_slash_prefilter_rejects_far_away_wires() {
    let curve = [
        egui::pos2(0.0, 0.0),
        egui::pos2(10.0, 0.0),
        egui::pos2(20.0, 0.0),
        egui::pos2(30.0, 0.0),
    ];
    assert!(curve_maybe_cut(
        &curve,
        egui::pos2(15.0, -5.0),
        egui::pos2(15.0, 5.0)
    ));
    assert!(!curve_maybe_cut(
        &curve,
        egui::pos2(100.0, -5.0),
        egui::pos2(100.0, 5.0)
    ));
}

// ---- 连线规则 ----

/// 造一张只有若干节点、没有连线的空画布，节点的端口按默认参数算好。
fn canvas(kind_ids: &[&str]) -> Graph {
    let kinds = crate::catalog::all();
    let mut nodes = Vec::new();
    for (i, id) in kind_ids.iter().enumerate() {
        let kind = kinds
            .iter()
            .find(|kind| kind.kind.id == *id)
            .unwrap_or_else(|| panic!("没有「{id}」这个节点"));
        let node = Node::new(
            new_id(),
            egui::pos2(i as f32 * 400.0, 0.0),
            kind.kind.id.clone(),
            kind.kind.name.clone(),
            kind.defaults.clone(),
        );
        nodes.push(node);
    }
    Graph {
        nodes,
        wires: Vec::new(),
        view: Viewport {
            pan: egui::Vec2::ZERO,
            zoom: 1.0,
        },
        revision: 0,
        check_revision: 0,
        tool_revision: None,
        ports_revision: None,
        selected: std::collections::BTreeSet::new(),
        interaction: Interaction::default(),
        textures: Textures::default(),
        anim: Anim::default(),
        pending_run: None,
        clipboard: Vec::new(),
        notes: HashMap::new(),
        run_marks_hidden: false,
        downloads: Downloads::default(),
        picker: dialog::Picker::default(),
    }
}

fn resolve_ports(graph: &mut Graph) {
    let workflow = graph.to_workflow(&Workflow::new("端口快照"));
    let resolved = starrytools_core::engine::resolve(&workflow);
    graph.sync_resolved_ports(graph.check_revision(), &resolved);
}

fn checked_ports(
    kind: &str,
    params: &Params,
) -> (
    Vec<starrytools_core::model::node_kind::PortDef>,
    Vec<starrytools_core::model::node_kind::PortDef>,
) {
    let mut graph = canvas(&[kind]);
    graph.nodes[0].params = params.clone();
    refresh_ports(&mut graph.nodes[0]);
    resolve_ports(&mut graph);
    (
        graph.nodes[0].inputs.clone(),
        graph.nodes[0].outputs.clone(),
    )
}

fn out(node: usize, port: usize) -> PortRef {
    PortRef {
        node,
        port,
        input: false,
    }
}

fn input(node: usize, port: usize) -> PortRef {
    PortRef {
        node,
        port,
        input: true,
    }
}

/// 从输出端口拉到输入端口，应当接上 —— 并在两端记下正确的端口 id。
#[test]
fn an_output_links_to_an_input() {
    let mut graph = canvas(&["read", "crop_image"]);
    let from = out(0, 0);
    let to = input(1, 1);

    assert!(graph.can_link(from, to), "输出应当能接到同类输入上");
    graph.link(from, to);

    assert_eq!(graph.wires.len(), 1);
    assert_eq!(graph.wires[0].from, 0);
    assert_eq!(graph.wires[0].to, 1);
    // 记的是**端口 id**，不是下标 —— 端口会随参数增减。
    assert_eq!(graph.wires[0].from_port, graph.nodes[0].outputs[0].id);
    assert_eq!(graph.wires[0].to_port, graph.nodes[1].inputs[1].id);
    assert_eq!(graph.revision, 1, "接上一条线要算改动");
    assert!(graph.is_selected(1), "接完顺手选上落点那个节点");
}

/// 一个节点不能给自己接线。
#[test]
fn a_node_cannot_link_to_itself() {
    let graph = canvas(&["crop_image"]);
    assert!(!graph.can_link(out(0, 0), input(0, 0)), "不能自己接自己");
}

/// 必须一头输入一头输出。
#[test]
fn a_link_needs_one_input_and_one_output() {
    let graph = canvas(&["read", "crop_image"]);
    assert!(!graph.can_link(out(0, 0), out(1, 0)), "两个输出接不上");
    assert!(!graph.can_link(input(1, 0), input(1, 0)), "两个输入接不上");
    assert!(
        !graph.can_link(out(0, 0), input(1, 9)),
        "不存在的端口接不上（不能越界崩掉）"
    );
}

/// 一个输入端口只接一条线，第二条要被拒。
#[test]
fn an_input_port_takes_only_one_wire() {
    let mut graph = canvas(&["read", "read", "crop_image"]);
    graph.link(out(0, 0), input(2, 1));
    assert_eq!(graph.wires.len(), 1);

    assert!(
        !graph.can_link(out(1, 0), input(2, 1)),
        "输入端口已经被占了，第二条该被拒"
    );
}

#[test]
fn exec_outputs_are_single_and_exec_inputs_allow_multiple_sources() {
    let mut graph = canvas(&["sequence", "sequence", "sequence"]);
    graph.link(out(0, 0), input(2, 0));
    assert!(!graph.can_link(out(0, 0), input(1, 0)));
    assert!(graph.can_link(out(1, 0), input(2, 0)));
    graph.link(out(1, 0), input(2, 0));
    assert_eq!(graph.wires.len(), 2);
}

#[test]
fn variable_nodes_live_in_their_category_and_accept_multiple_sets() {
    let kinds = crate::catalog::all();
    for id in ["literal_number", "literal_text", "literal_bool"] {
        let kind = kinds.iter().find(|kind| kind.kind.id == id).unwrap();
        assert_eq!(kind.kind.category, "变量");
        assert!(!kind.pure || kind.kind.inputs.iter().any(|p| p.id == "set"));
    }
    let mut graph = canvas(&["literal_number", "literal_number", "literal_number"]);
    assert_eq!(graph.nodes[0].inputs[0].label, "Set");
    assert_eq!(graph.nodes[0].outputs[0].label, "Get");
    graph.link(out(0, 0), input(1, 0));
    assert!(
        graph.can_link(out(2, 0), input(1, 0)),
        "Set 口应该允许接多个上游"
    );
    graph.link(out(2, 0), input(1, 0));
    assert_eq!(graph.wires.len(), 2);
}

#[test]
fn variable_category_contains_literals_operation_and_set() {
    let kinds = crate::catalog::all();
    for id in [
        "literal_text",
        "literal_number",
        "literal_bool",
        "operation",
        "set",
    ] {
        let kind = kinds.iter().find(|kind| kind.kind.id == id).unwrap();
        assert_eq!(kind.kind.category, "变量", "{id}");
    }
    let set = kinds.iter().find(|kind| kind.kind.id == "set").unwrap();
    assert!(!set.pure, "Set 必须走执行线");
    assert!(set.kind.inputs.iter().any(|port| port.exec));
    assert!(set.kind.outputs.iter().any(|port| port.exec));
}

#[test]
fn hovering_a_port_requests_animation_frames() {
    let mut graph = canvas(&["literal_number"]);
    graph.interaction.hovered_port = Some(input(0, 0));
    assert!(graph.is_animating(), "悬停光晕需要持续重绘");
    graph.interaction.hovered_port = None;
    assert!(!graph.is_animating());
}

#[test]
fn successful_run_states_are_written_back_to_variable_params() {
    let kinds = crate::catalog::all();
    let mut graph = canvas(&["literal_number"]);
    let id = graph.nodes[0].id.clone();
    let before = graph.check_revision();
    assert!(graph.apply_variable_states(
        &std::collections::BTreeMap::from([(id, serde_json::json!(5.0))]),
        &kinds,
    ));
    assert_eq!(graph.nodes[0].params["value"], serde_json::json!(5.0));
    assert!(graph.check_revision() > before);
    // 值没变时不应继续把画布标成脏。
    assert!(!graph.apply_variable_states(
        &std::collections::BTreeMap::from([(graph.nodes[0].id.clone(), serde_json::json!(5.0))]),
        &kinds,
    ));
}

#[test]
fn exec_never_connects_to_data_even_any_or_void() {
    use starrytools_core::model::port_type::PortType;
    let mut graph = canvas(&["sequence", "operation"]);
    let operand = graph.nodes[1]
        .inputs
        .iter()
        .position(|port| !port.exec)
        .expect("运算节点有数据输入");
    for ty in [PortType::Any, PortType::Void] {
        graph.nodes[1].inputs[operand].ty = ty;
        assert!(!graph.can_link(out(0, 0), input(1, operand)));
        assert!(!graph.can_link(input(1, operand), out(0, 0)));
    }
}

/// 类型对不上就接不上：文本输出进不了图像输入。
#[test]
fn a_type_mismatch_is_rejected() {
    let graph = canvas(&["literal_text", "crop_image"]);
    assert_eq!(graph.nodes[0].outputs[0].badge(), "TXT", "文本节点输出文本");
    assert!(
        !graph.can_link(out(0, 0), input(1, 0)),
        "文本不该接到图像输入上"
    );
}

#[test]
fn declared_implicit_conversions_are_linkable_in_both_directions() {
    let number_to_text = canvas(&["literal_number", "literal_text"]);
    assert!(number_to_text.can_link(out(0, 0), input(1, 0)));

    let text_to_number = canvas(&["literal_text", "literal_number"]);
    assert!(text_to_number.can_link(out(0, 0), input(1, 0)));
}

#[test]
fn marquee_touches_one_way_and_contains_the_other() {
    let kinds = crate::catalog::all();
    let mut graph = canvas(&["read", "read"]);
    let first = graph.screen_rect(&kinds, 0);
    let second = graph.screen_rect(&kinds, 1);

    // 起点在左侧：碰到就算。
    graph.interaction.marquee = Some(Marquee {
        start: egui::pos2(first.left() - 5.0, first.top() - 5.0),
        current: egui::pos2(second.right() + 5.0, second.bottom() + 5.0),
        base: Default::default(),
    });
    graph.update_marquee_selection(&kinds);
    assert!(graph.is_selected(0) && graph.is_selected(1));

    // 起点在右侧：只选完全包含；把终点拖进第一个节点内部，两个都不完整。
    graph.interaction.marquee = Some(Marquee {
        start: egui::pos2(second.right() + 5.0, second.bottom() + 5.0),
        current: egui::pos2(first.left() + 30.0, first.center().y),
        base: Default::default(),
    });
    graph.update_marquee_selection(&kinds);
    assert!(graph.selected.is_empty());

    // 从右往左完整框住两个节点。
    graph.interaction.marquee = Some(Marquee {
        start: egui::pos2(second.right() + 5.0, second.bottom() + 5.0),
        current: egui::pos2(first.left() - 5.0, first.top() - 5.0),
        base: Default::default(),
    });
    graph.update_marquee_selection(&kinds);
    assert!(graph.is_selected(0) && graph.is_selected(1));
}

#[test]
fn ctrl_marquee_keeps_the_selection_it_started_with() {
    let kinds = crate::catalog::all();
    let mut graph = canvas(&["read", "read", "read"]);
    graph.select_only(0);
    let third = graph.screen_rect(&kinds, 2);

    // 框只罩住第三个；base 里已有的第一个应该保留，没碰到的第二个不该被选中。
    let base = graph.selected.clone();
    graph.interaction.marquee = Some(Marquee {
        start: egui::pos2(third.left() - 5.0, third.top() - 5.0),
        current: egui::pos2(third.right() + 5.0, third.bottom() + 5.0),
        base,
    });
    graph.update_marquee_selection(&kinds);
    assert!(graph.is_selected(0) && graph.is_selected(2));
    assert!(!graph.is_selected(1));
}

#[test]
fn bringing_a_selected_node_to_front_keeps_the_whole_selection() {
    let mut graph = canvas(&["read", "read", "read"]);
    let ids: Vec<_> = graph.nodes.iter().map(|node| node.id.clone()).collect();
    graph.select_only(0);
    graph.toggle_selection(2);

    let moved = graph.bring_to_front(0);

    assert_eq!(moved, 2);
    assert_eq!(graph.nodes[2].id, ids[0], "被拖的节点到了最上层");
    assert!(graph.is_selected(1) && graph.is_selected(2));
    assert_eq!(
        graph.nodes[1].id, ids[2],
        "另一个选中节点的下标也要跟着换算"
    );
    assert!(!graph.is_selected(0));
    assert_eq!(graph.nodes[0].id, ids[1], "未选中节点不受影响");
}

#[test]
fn ctrl_toggle_and_group_move_follow_the_selection_set() {
    let mut graph = canvas(&["read", "read", "read"]);
    graph.select_only(0);
    graph.toggle_selection(1);
    assert!(graph.is_selected(0) && graph.is_selected(1));
    graph.toggle_selection(0);
    assert!(!graph.is_selected(0) && graph.is_selected(1));

    let before: Vec<_> = graph.nodes.iter().map(|node| node.pos).collect();
    graph.select_only(0);
    graph.toggle_selection(2);
    graph.move_selected_by(egui::vec2(10.0, 5.0));
    assert_eq!(graph.nodes[0].pos, before[0] + egui::vec2(10.0, 5.0));
    assert_eq!(graph.nodes[2].pos, before[2] + egui::vec2(10.0, 5.0));
    assert_eq!(graph.nodes[1].pos, before[1], "未选中节点不该移动");
}

// ---- 节点右键菜单的两件事 ----

/// 删掉一个节点，接在它身上的连线要一起断，剩下连线的下标要跟着往前挪。
#[test]
fn deleting_a_node_drops_its_wires_and_reindexes_the_rest() {
    let mut graph = canvas(&["read", "read", "crop_image"]);
    graph.link(out(0, 1), input(2, 0));
    graph.link(out(1, 0), input(2, 1));
    assert_eq!(graph.wires.len(), 2);

    graph.delete_node(0);

    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.wires.len(), 1, "接在被删节点上的连线要一起断掉");
    // 原 1→2 的那条，两个下标都该减一。
    assert_eq!(graph.wires[0].from, 0, "剩下的连线下标要往前挪");
    assert_eq!(graph.wires[0].to, 1);
    assert!(graph.is_selected(1), "选中下标也要跟着往前挪");
    assert_eq!(graph.selected.len(), 1);
    // 挪完还得指得对端口 —— 指错就会接到别的端口上去。
    assert_eq!(graph.wires[0].from_port, graph.nodes[0].outputs[0].id);
    assert_eq!(graph.wires[0].to_port, graph.nodes[1].inputs[1].id);
}

/// 复制一个节点：新 id、参数照搬、端口重算。
#[test]
fn duplicating_a_node_gives_it_a_new_id_and_keeps_its_params() {
    let mut graph = canvas(&["crop_image"]);
    graph.nodes[0]
        .params
        .insert("mode".into(), serde_json::json!("size"));
    refresh_ports(&mut graph.nodes[0]);
    let want = graph.nodes[0].params.clone();

    graph.duplicate_node(0);

    assert_eq!(graph.nodes.len(), 2);
    assert_ne!(graph.nodes[0].id, graph.nodes[1].id, "复制体要有自己的 id");
    assert_eq!(graph.nodes[1].params, want, "参数要一起复制过去");
    assert_eq!(graph.nodes[1].outputs.len(), graph.nodes[0].outputs.len());
    assert!(graph.is_selected(1), "复制完选中新的那个");
}

// ---- 字面量节点与参数端口 ----

#[test]
fn literals_are_variables_with_get_and_set_but_no_exec() {
    let kinds = crate::catalog::all();
    for id in ["literal_text", "literal_number", "literal_bool"] {
        let graph = canvas(&[id]);
        assert_eq!(graph.nodes[0].inputs.len(), 1);
        assert_eq!(graph.nodes[0].inputs[0].id, "set");
        assert!(graph.nodes[0].inputs[0].multi);
        assert!(!graph.nodes[0].inputs[0].exec);
        assert_eq!(graph.nodes[0].outputs.len(), 1);
        assert_eq!(graph.nodes[0].outputs[0].id, "out");
        assert!(!graph.nodes[0].outputs[0].exec);
        assert!(graph.can_run_node(&kinds, 0));
    }
    let graph = canvas(&["start", "sequence", "iterate", "read", "operation"]);
    for i in 0..5 {
        assert!(!graph.can_run_node(&kinds, i));
    }
}

/// 「运算」的输入端口跟着运算方式走：数值模式是 NUM，拼接模式是 TXT。
#[test]
fn operation_ports_follow_the_selected_mode() {
    let mut params = crate::catalog::all()
        .into_iter()
        .find(|kind| kind.kind.id == "operation")
        .expect("该有「运算」节点")
        .defaults;
    let (inputs, _) = crate::catalog::ports_for("operation", &params);
    let data: Vec<_> = inputs.iter().filter(|port| !port.exec).collect();
    assert_eq!(data.len(), 2, "{inputs:?}");
    assert!(
        data.iter().all(|port| port.badge() == "NUM"),
        "默认是数值运算：{inputs:?}"
    );

    params.insert("op".into(), serde_json::json!("concat"));
    let (inputs, _) = crate::catalog::ports_for("operation", &params);
    let data: Vec<_> = inputs.iter().filter(|port| !port.exec).collect();
    assert_eq!(data.len(), 2, "{inputs:?}");
    assert!(
        data.iter().all(|port| port.badge() == "TXT"),
        "切到拼接后端口该变成文本：{inputs:?}"
    );
}

/// 动态输出节点：「顺序执行」的端口跟着 `outs` 参数走；它还带一个空输入。
#[test]
fn dynamic_output_nodes_expose_numbered_ports() {
    let kinds = crate::catalog::all();
    let seq = kinds
        .iter()
        .find(|kind| kind.kind.id == "sequence")
        .expect("该有「顺序执行」");
    assert_eq!(seq.kind.category, "逻辑");

    let mut params = seq.defaults.clone();
    params.insert("outs".into(), serde_json::json!(2.0));
    let (inputs, outputs) = checked_ports("sequence", &params);
    assert_eq!(
        outputs.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["out1", "out2"]
    );
    assert!(outputs.iter().all(|port| port.exec), "每个输出都该是 Exec");
    assert_eq!(
        outputs
            .iter()
            .map(|port| port.label.as_str())
            .collect::<Vec<_>>(),
        vec!["1", "2"],
        "每个 Exec 输出都带自己的序号"
    );
    assert_eq!(inputs.len(), 1, "只该有一个空输入");
    assert_eq!(inputs[0].badge(), "-");

    // 「遍历」：一个循环体输出 + 可选的结束；输入是列表。
    let iter = kinds.iter().find(|kind| kind.kind.id == "iterate").unwrap();
    let (inputs, outputs) = checked_ports("iterate", &iter.defaults);
    assert_eq!(
        outputs.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["body", "item", "finally"]
    );
    assert_eq!(inputs[1].badge(), "[ANY]", "列表输入徽标带方括号");
}

#[test]
fn repeat_node_exposes_body_index_and_completion() {
    let kinds = crate::catalog::all();
    let repeat = kinds
        .iter()
        .find(|kind| kind.kind.id == "repeat")
        .expect("该有「重复执行」");
    assert_eq!(repeat.kind.category, "逻辑");
    let (inputs, outputs) = checked_ports("repeat", &repeat.defaults);
    assert!(inputs.iter().any(|port| port.exec), "需要执行输入");
    assert!(outputs.iter().any(|port| port.id == "body" && port.exec));
    assert!(outputs.iter().any(|port| port.id == "index" && !port.exec));
    assert!(outputs.iter().any(|port| port.id == "finally" && port.exec));
    assert!(repeat.kind.params.iter().any(|param| param.id == "times"));
}

/// 动态输出挂在参数栏里，顶部端口列里不再有输出 —— 卡片高度不随输出个数涨。
#[test]
fn sequence_branch_names_are_editable_defaults_not_semantics() {
    let kinds = crate::catalog::all();
    let seq = kinds
        .iter()
        .find(|kind| kind.kind.id == "sequence")
        .unwrap();
    assert_eq!(seq.defaults["branch1"], serde_json::json!("分支 1"));

    let mut graph = canvas(&["sequence"]);
    let param = seq.kind.params.first().unwrap().clone();
    assert!(graph.handle_control(
        0,
        &param,
        Some(crate::ui::controls::ControlEvent::Changed(
            serde_json::json!(3)
        )),
    ));
    assert_eq!(
        graph.nodes[0].params["branch2"],
        serde_json::json!("分支 2")
    );
    assert_eq!(
        graph.nodes[0].params["branch3"],
        serde_json::json!("分支 3")
    );
    assert_eq!(
        graph.nodes[0].params["branch1"],
        serde_json::json!("分支 1")
    );
}

#[test]
fn dynamic_outputs_live_in_the_params_panel() {
    let kinds = crate::catalog::all();
    let mut params = kinds
        .iter()
        .find(|kind| kind.kind.id == "sequence")
        .unwrap()
        .defaults
        .clone();
    params.insert("outs".into(), serde_json::json!(3.0));
    let (_, outputs) = checked_ports("sequence", &params);
    assert_eq!(
        outputs
            .iter()
            .map(|port| port.panel_row)
            .collect::<Vec<_>>(),
        vec![Some(0), Some(1), Some(2)],
        "每个输出都该挂在参数栏的对应行上"
    );

    let mut graph = canvas(&["sequence"]);
    graph.nodes[0]
        .params
        .insert("outs".into(), serde_json::json!(3.0));
    refresh_ports(&mut graph.nodes[0]);
    graph.touch_content();
    resolve_ports(&mut graph);
    assert_eq!(graph.nodes[0].outputs.len(), 3);
    // 顶部端口列只剩那一个空输入，不受 3 个输出影响。
    assert_eq!(node_body_top(&graph.nodes[0]), HEADER_H + PORT_ROW_H);
}

/// 普通节点里「能被上游喂」的参数自动多一个参数端口，并标好它属于哪个参数。
/// 【关键】它不算进端口列 —— 端口列只数非参数端口，否则圆点会和端口挤在一起。
#[test]
fn param_ports_are_derived_marked_and_kept_out_of_the_port_column() {
    let kind = crate::catalog::all()
        .into_iter()
        .find(|kind| kind.kind.id == "upscale")
        .expect("该有「缩放图像」");
    let (inputs, _) = crate::catalog::ports_for("upscale", &kind.defaults);

    let percent = inputs
        .iter()
        .find(|port| port.id == "param:percent")
        .expect("数字参数该多一个端口");
    assert!(percent.is_param());
    assert_eq!(percent.param.as_deref(), Some("percent"));
    assert!(!percent.required, "参数端口永远是可选的");
    // Select 的合法值是一张固定表，不能让上游乱喂 —— 不该有端口。
    assert!(
        !inputs.iter().any(|port| port.id == "param:filter"),
        "下拉参数不该有端口"
    );

    let graph = canvas(&["upscale"]);
    assert_eq!(
        node_body_top(&graph.nodes[0]),
        HEADER_H + PORT_ROW_H * 2.0,
        "端口列只数非参数端口（这里只有一个图像输入）"
    );
}

/// 参数端口要落在它那一行参数的标签上（浅蓝小圆点），而不是掉进端口列。
/// percent 是「缩放图像」的第一个参数，圆点该贴左缘、落在参数区最上面那行。
#[test]
fn a_param_port_sits_on_its_param_row() {
    let kinds = crate::catalog::all();
    let graph = canvas(&["upscale"]);
    let k = graph.nodes[0]
        .inputs
        .iter()
        .position(|port| port.param.as_deref() == Some("percent"))
        .expect("缩放图像该有 percent 的参数端口");

    let card = graph.flow_rect(&kinds, 0);
    let port = graph.in_port(&kinds, 0, k);
    let offset = graph
        .param_port_offset(&kinds, 0, "percent")
        .expect("percent 该有定位偏移");
    let params_top = node_body_top(&graph.nodes[0]) + PARAMS_GAP;

    assert_eq!(port.x, card.left(), "参数端口该贴卡片左缘");
    assert!(
        offset >= params_top,
        "参数端口该在参数区里（端口列下方）：{offset} < {params_top}"
    );
    assert!(
        offset - params_top <= PARAM_LABEL_H,
        "percent 是第一个参数，圆点该在最上面那行：偏移 {}",
        offset - params_top
    );
    assert!(port.y < card.bottom(), "参数端口不能掉出卡片");
}

/// 灰底参数框的上沿要和里面第一个控件对得上 —— 参数端口不能撑高端口区。
/// 曾经就错在这里：`node_body_top` 把参数端口也算进了行数，于是每多一个
/// 参数端口，灰框就比控件低一整行。
#[test]
fn the_params_panel_lines_up_with_where_the_controls_are_drawn() {
    let kinds = crate::catalog::all();
    let graph = canvas(&["upscale"]);
    let kind = kinds.iter().find(|kind| kind.kind.id == "upscale").unwrap();

    // 灰底参数框的上沿（相对卡片顶部）。
    let panel_top = node_body_top(&graph.nodes[0]);
    // 实际绘制用的第一个参数块的顶部 —— 应当正好在参数框上沿下方 PARAMS_GAP。
    let slots = param_slots(&graph.nodes[0], kind, &graph.notes);
    let first_top = slots[0].label_y.unwrap_or(slots[0].control_y);
    assert_eq!(
        first_top - panel_top,
        PARAMS_GAP,
        "参数框上沿到第一个参数块之间应当正好是 PARAMS_GAP"
    );
    // upscale 只声明了一个图像输入；参数端口 `param:percent` 不该算进端口列。
    assert_eq!(
        panel_top,
        HEADER_H + PORT_ROW_H * 2.0,
        "端口区高度只按声明端口算，参数端口不占一席"
    );
}

/// 刚拖出来的节点要**以指针为中心**落下来 —— 落点用的高度和画卡片用的高度必须是同一个
/// （`add_node_at` 现在直接复用 `height_of_node`，所以这条是结构上的保证，不再是两条求和路径）。
#[test]
fn a_dropped_node_is_centred_on_the_pointer() {
    let kinds = crate::catalog::all();
    for id in [
        "read",
        "convert_image",
        "compress_image",
        "crop_image",
        "upscale",
        "rename",
        "save_output",
        "literal_text",
        // 本地没模型时卡片会多出一块下载面板 —— 落点也得把它算进去。
        #[cfg(feature = "onnx")]
        "background_removal",
    ] {
        let mut graph = canvas(&[]);
        let kind = kinds.iter().find(|kind| kind.kind.id == id).unwrap();
        // `canvas` 的 pan 是 0、zoom 是 1，所以流坐标就是屏幕坐标。
        let screen = egui::pos2(300.0, 200.0);
        graph.add_node_at(screen, kind, 0.0);
        let node = graph.nodes.last().unwrap();
        let height = height_of_node(&kinds, node, &graph.notes);
        let centre = egui::pos2(node.pos.x + NODE_W * 0.5, node.pos.y + height * 0.5);
        assert!(
            (centre - screen).length() < 0.001,
            "{id}：落点 {centre:?} 偏离指针 {screen:?}"
        );
    }
}

/// 字面量的输出能接到参数端口上 —— 这是「参数改从上游取值」的入口。
#[test]
fn a_literal_links_into_a_param_port() {
    let mut graph = canvas(&["literal_text", "rename"]);
    let to = graph.nodes[1]
        .inputs
        .iter()
        .position(|port| port.param.as_deref() == Some("name"))
        .expect("重命名该有文件名参数端口");

    assert!(
        graph.can_link(out(0, 0), input(1, to)),
        "文本该能接到文本参数上"
    );
    graph.link(out(0, 0), input(1, to));
    assert_eq!(graph.wires.len(), 1);
    assert_eq!(graph.wires[0].to_port, graph.nodes[1].inputs[to].id);
}

/// 取色器的两个颜色空间互为逆运算 —— 不然拖一下取色区颜色就偏了。
#[test]
fn hsv_round_trips_through_its_inverse() {
    for rgba in [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [18, 52, 86, 255],
        [200, 200, 200, 255],
        [0, 0, 0, 255],
        [255, 255, 255, 255],
    ] {
        let (h, s, v) = rgb_to_hsv(rgba[0], rgba[1], rgba[2]);
        let (r, g, b) = hsv_to_rgb(h, s, v);
        assert_eq!([r, g, b], [rgba[0], rgba[1], rgba[2]], "{rgba:?}");
    }
}

/// 颜色字符串和 RGBA 互为逆运算；坏值不 panic。
#[test]
fn color_strings_round_trip() {
    for value in ["#000000", "#ffffff", "#123456", "#12345678"] {
        assert_eq!(color_string(color_rgba(value)), value);
    }
    assert_eq!(color_rgba("transparent"), [0, 0, 0, 0]);
    assert_eq!(color_string(color_rgba("transparent")), "#00000000");
    assert_eq!(color_rgba("不是颜色"), [0, 0, 0, 255]);
}

#[test]
fn unknown_nodes_and_legacy_port_ids_survive_loading() {
    let mut graph = canvas(&["read", "rename"]);
    graph.nodes[0].kind = "removed_tool".into();
    graph.nodes[0].params = Params::from_iter([(
        "legacy_value".into(),
        serde_json::json!({"nested": [1, "kept"]}),
    )]);
    refresh_ports(&mut graph.nodes[0]);
    graph.wires.push(Wire {
        from: 0,
        from_port: "legacy-output".into(),
        to: 1,
        to_port: "legacy-input".into(),
        id: Wire::make_id(
            &graph.nodes[0].id,
            "legacy-output",
            &graph.nodes[1].id,
            "legacy-input",
        ),
    });
    let workflow = graph.to_workflow(&Workflow::new("旧存档"));
    let back = Graph::from_workflow(&workflow, &crate::catalog::all());

    assert_eq!(back.nodes[0].title, "removed_tool");
    assert!(back.nodes[0].inputs.is_empty());
    assert!(back.nodes[0].outputs.is_empty());
    assert_eq!(back.revision, 0);
    assert_eq!(back.check_revision(), 0);
    assert_eq!(
        serde_json::to_value(back.to_workflow(&workflow)).unwrap(),
        serde_json::to_value(workflow).unwrap(),
    );
}

#[test]
fn loading_a_workflow_fills_newly_added_default_params() {
    let mut workflow = Workflow::new("旧变换");
    workflow
        .nodes
        .push(starrytools_core::model::workflow::NodeInstance {
            id: "tf".into(),
            kind: "transform_image".into(),
            position: starrytools_core::model::workflow::Position::default(),
            params: Params::from_iter([("flipHorizontal".into(), serde_json::json!(true))]),
        });
    let graph = Graph::from_workflow(&workflow, &crate::catalog::all());
    let params = &graph.nodes[0].params;
    assert_eq!(params["mode"], serde_json::json!("flip"));
    assert_eq!(params["flipHorizontal"], serde_json::json!(true));
    assert_eq!(params["stretchWidth"], serde_json::json!(100.0));
}

#[test]
fn restoring_a_node_rebuilds_ports_without_run_state() {
    let mut graph = canvas(&["convert_image"]);
    let node = &mut graph.nodes[0];
    node.preview = true;
    node.palette = Some(vec![[1, 2, 3, 255]]);
    node.run = Some(NodeRun {
        status: starrytools_core::engine::NodeStatus::Ok,
        ms: 12,
        error: None,
        warnings: vec!["not persisted".into()],
        file: Some("result.png".into()),
    });
    let workflow = graph.to_workflow(&Workflow::new("瞬态"));
    let back = Graph::from_workflow(&workflow, &crate::catalog::all());
    let restored = &back.nodes[0];

    assert_eq!(restored.id, graph.nodes[0].id);
    assert_eq!(restored.params, graph.nodes[0].params);
    assert_eq!(
        serde_json::to_value(&restored.inputs).unwrap(),
        serde_json::to_value(&graph.nodes[0].inputs).unwrap(),
    );
    assert_eq!(
        serde_json::to_value(&restored.outputs).unwrap(),
        serde_json::to_value(&graph.nodes[0].outputs).unwrap(),
    );
    assert!(!restored.preview);
    assert!(restored.palette.is_none());
    assert!(restored.run.is_none());
    assert!(restored.step.is_none());
}

#[test]
fn failed_previews_are_cached_until_the_url_changes_or_run_resets() {
    use starrytools_core::engine::{NodeRunResult, NodeStatus, PortResult};
    use starrytools_core::image_io::{EncodeOptions, ImageValue};
    use starrytools_core::model::port_type::{ImageFormat, PortType};

    let mut graph = canvas(&["read"]);
    let ctx = egui::Context::default();
    let node = &graph.nodes[0];
    let mut result = NodeRunResult {
        node_id: node.id.clone(),
        kind: node.kind.clone(),
        name: node.title.clone(),
        status: NodeStatus::Ok,
        error: None,
        warnings: vec!["result remains visible".into()],
        elapsed_ms: 7,
        outputs: vec![PortResult {
            port_id: "out".into(),
            label: "图像".into(),
            ty: PortType::Image(ImageFormat::Png),
            summary: "preview".into(),
            preview: Some("data:image/png;base64,invalid".into()),
            path: Some("output.png".into()),
            palette: None,
        }],
    };
    graph.apply_result(&ctx, &result);
    assert!(!graph.nodes[0].preview);
    assert_eq!(
        graph.textures.failed.get(&result.node_id),
        result.outputs[0].preview.as_ref()
    );
    assert_eq!(
        graph.nodes[0].run.as_ref().unwrap().file.as_deref(),
        Some("output.png")
    );

    graph.apply_result(&ctx, &result);
    assert_eq!(graph.textures.failed.len(), 1);
    assert!(graph.textures.card.is_empty());

    result.outputs[0].preview = Some("data:image/png;base64,also-invalid".into());
    graph.apply_result(&ctx, &result);
    assert_eq!(
        graph.textures.failed.get(&result.node_id),
        result.outputs[0].preview.as_ref()
    );

    let image = ImageValue::from_image(
        ImageFormat::Png,
        std::sync::Arc::new(image::DynamicImage::ImageRgba8(image::RgbaImage::new(1, 1))),
        EncodeOptions::default(),
    )
    .unwrap();
    result.outputs[0].preview = Some(image.preview_data_url(1).unwrap());
    graph.apply_result(&ctx, &result);
    assert!(graph.nodes[0].preview);
    assert!(graph.textures.failed.is_empty());
    let texture_id = graph.textures.card[&result.node_id].id();
    graph.apply_result(&ctx, &result);
    assert_eq!(graph.textures.card[&result.node_id].id(), texture_id);

    graph.reset_run_marks();
    assert!(graph.textures.card.is_empty());
    assert!(graph.textures.failed.is_empty());
    assert!(!graph.nodes[0].preview);
    assert!(graph.nodes[0].run.is_none());

    result.outputs[0].preview = Some("data:image/png;base64,invalid".into());
    graph.apply_result(&ctx, &result);
    assert_eq!(graph.textures.failed.len(), 1);
    graph.reset_run_marks();
    assert!(graph.textures.failed.is_empty());
}

#[test]
fn resolved_exec_and_layout_metadata_changes_cancel_port_interactions() {
    let mut graph = canvas(&["operation"]);
    let mut resolved =
        starrytools_core::engine::resolve(&graph.to_workflow(&Workflow::new("元数据")));
    let here = input(0, 0);
    graph.interaction.hovered_port = Some(here);
    graph.interaction.connect = Some(Connect {
        from: here,
        to: Pos2::ZERO,
        target: None,
    });
    let before = resolved.nodes[0].inputs[0].exec;
    let port = &mut resolved.nodes[0].inputs[0];
    port.exec = !before;
    port.label = "执行".into();
    port.required = false;
    port.hint = Some("hint".into());
    port.param = None;
    port.panel_input_row = None;
    graph.sync_resolved_ports(0, &resolved);
    assert_eq!(graph.nodes[0].inputs[0].exec, !before);
    assert_eq!(graph.nodes[0].inputs[0].label, "执行");
    assert!(graph.interaction.connect.is_none());
    assert!(graph.interaction.hovered_port.is_none());
    assert_eq!(graph.revision, 0);
    assert_eq!(graph.check_revision(), 0);
}

#[test]
fn exec_ports_precede_data_even_when_declared_last() {
    let kinds = crate::catalog::all();
    let mut graph = canvas(&["crop_image"]);
    graph.nodes[0].inputs.reverse();
    graph.nodes[0].outputs.reverse();
    let rect = graph.flow_rect(&kinds, 0);
    for (input_side, ports) in [
        (true, &graph.nodes[0].inputs),
        (false, &graph.nodes[0].outputs),
    ] {
        let exec = ports.iter().position(|port| port.exec).unwrap();
        let data = ports
            .iter()
            .position(|port| !port.exec && !port.is_param() && port.panel_row.is_none())
            .unwrap();
        let point = graph.port_pos(
            &kinds,
            PortRef {
                node: 0,
                port: exec,
                input: input_side,
            },
        );
        let data_point = graph.port_pos(
            &kinds,
            PortRef {
                node: 0,
                port: data,
                input: input_side,
            },
        );
        assert_eq!(point.y, rect.top() + HEADER_H + PORT_ROW_H * 0.5);
        assert!(data_point.y > point.y);
    }
}
