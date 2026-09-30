/** 画布上的节点卡片。一个组件渲染所有工具 —— 形状全来自后端给的元数据。 */

import { Handle, Position, type NodeProps } from '@xyflow/react';
import { Download, FolderSearch, Play, X } from 'lucide-react';
import { memo } from 'react';

import { issuesForPort, linkedPorts } from '../lib/graph';
import { countRender } from '../lib/dev-render';
import { isVisible } from '../lib/params';
import { portBadge, portInk } from '../lib/ports';
import type { NodeStatus, PortDef, ToolNode } from '../lib/types';
import { useStore } from '../state/store';
import { Button } from '../ui/Button';
import { ParamField } from './ParamField';

const STATUS_TEXT: Record<NodeStatus, string> = {
  ok: '完成',
  failed: '出错',
  skipped: '跳过',
};

function ToolNodeCard({ id, data, selected }: NodeProps<ToolNode>) {
  countRender('节点卡片');
  const kindById = useStore((state) => state.kindById);
  const resolved = useStore((state) => state.resolved);
  const run = useStore((state) => state.report?.nodes.find((node) => node.nodeId === id));
  const preview = useStore((state) => state.previews[id]);
  const fileInfo = useStore((state) => state.fileInfos[id]);
  const pulse = useStore((state) => state.pulse);
  const entering = useStore((state) => state.justAdded === id);
  const updateParam = useStore((state) => state.updateParam);
  const chooseFile = useStore((state) => state.chooseFile);
  const removeNode = useStore((state) => state.removeNode);
  const runTo = useStore((state) => state.run);
  const reveal = useStore((state) => state.reveal);
  const exportOutput = useStore((state) => state.exportOutput);

  // 连了哪些端口。查的是一张预先算好的表（见 lib/graph.ts 的 linkedPorts）。
  // 拼成字符串是为了让 zustand 用值比较，不引起无谓重渲染。
  const linkedInputs = useStore((state) => linkedPorts(state.edges)[id]?.inputs ?? '');
  const linkedOutputs = useStore((state) => linkedPorts(state.edges)[id]?.outputs ?? '');

  const node = resolved?.nodes.find((candidate) => candidate.nodeId === id);
  // 用归一后的 kind —— 老存档里写的可能是改过的旧 id，data.kind 查不到元数据。
  const kind = kindById[node?.kind ?? data.kind];
  const inputs = node?.inputs ?? [];
  const outputs = node?.outputs ?? [];
  const rowCount = Math.max(inputs.length, outputs.length);
  const connectedInputs = new Set(linkedInputs ? linkedInputs.split('|') : []);
  const connectedOutputs = new Set(linkedOutputs ? linkedOutputs.split('|') : []);

  const params = (kind?.params ?? []).filter((def) => isVisible(def, data.params));

  const pulseIndex = pulse ? pulse.order.indexOf(id) : -1;
  const pulseClass =
    !pulse || pulseIndex < 0 ? '' : pulseIndex < pulse.step ? 'is-swept' : 'is-live';

  const shown = run?.outputs.find((output) => output.preview);
  const file = run?.outputs.find((output) => output.path);

  return (
    <div
      className={[
        'node',
        selected ? 'is-selected' : '',
        pulseClass,
        entering ? 'is-entering' : '',
        run ? `is-${run.status}` : '',
      ]
        .filter(Boolean)
        .join(' ')}
    >
      <header className="node__head">
        <span className="node__heading">
          <span className="node__title">{kind?.name ?? data.kind}</span>
          {kind?.isSource && <span className="node__tag">起点</span>}
        </span>
        {run && (
          <span className={`node__state node__state--${run.status}`} title={run.error ?? undefined}>
            {STATUS_TEXT[run.status]}
            <em>{run.elapsedMs}ms</em>
          </span>
        )}
        <span className="node__tools">
          <Button
            size="icon-sm"
            variant="ghost"
            tooltip="运行至此"
            onClick={() => void runTo(id)}
          >
            <Play size={11} />
          </Button>
          <Button size="icon-sm" variant="ghost" tooltip="删除节点" onClick={() => removeNode(id)}>
            <X size={11} />
          </Button>
        </span>
      </header>

      {rowCount > 0 && (
        <div className="node__ports">
          {Array.from({ length: rowCount }, (_, index) => {
            const inputPort = inputs[index];
            const outputPort = outputs[index];
            return (
              <div className="port-row" key={index}>
                <div className="port-row__side">
                  {inputPort && (
                    <PortCell
                      port={inputPort}
                      direction="in"
                      linked={connectedInputs.has(inputPort.id)}
                      problems={issuesForPort(resolved, id, inputPort.id).map(
                        (issue) => issue.message,
                      )}
                    />
                  )}
                </div>
                <div className="port-row__side port-row__side--out">
                  {outputPort && (
                    <PortCell
                      port={outputPort}
                      direction="out"
                      linked={connectedOutputs.has(outputPort.id)}
                      problems={issuesForPort(resolved, id, outputPort.id).map(
                        (issue) => issue.message,
                      )}
                    />
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}

      {params.length > 0 && (
        <div className="node__params">
          {params.map((def) => (
            <ParamField
              key={def.id}
              def={def}
              value={data.params[def.id]}
              fileInfo={fileInfo}
              onChange={(value) => updateParam(id, def.id, value)}
              onPickFile={() => void chooseFile(id)}
            />
          ))}
        </div>
      )}

      {preview && (
        <div className="node__preview">
          <img src={preview} alt="" />
          {shown?.summary && <span className="node__caption">{shown.summary}</span>}
        </div>
      )}

      {run && run.warnings.length > 0 && (
        <ul className="node__warnings">
          {run.warnings.map((warning) => (
            <li key={warning}>{warning}</li>
          ))}
        </ul>
      )}

      {run?.error && <p className="node__error">{run.error}</p>}

      {file?.path && (
        <div className="node__actions">
          <Button
            size="icon-sm"
            variant="ghost"
            tooltip="在文件夹中显示"
            onClick={() => void reveal(file.path ?? '')}
          >
            <FolderSearch size={11} />
          </Button>
          <Button
            size="icon-sm"
            variant="ghost"
            tooltip="另存为…"
            onClick={() =>
              void exportOutput(file.path ?? '', (file.path ?? '').split(/[/\\]/).pop() ?? 'output.png')
            }
          >
            <Download size={11} />
          </Button>
          <span className="node__filename">{file.path.split(/[/\\]/).pop()}</span>
        </div>
      )}
    </div>
  );
}

function PortCell({
  port,
  direction,
  linked,
  problems,
}: {
  port: PortDef;
  direction: 'in' | 'out';
  linked: boolean;
  problems: string[];
}) {
  const ink = portInk(port.ty);
  const open = port.required && !linked;
  const className = [
    'port',
    `port--${direction}`,
    open ? 'is-open' : '',
    problems.length > 0 ? 'is-flagged' : '',
  ]
    .filter(Boolean)
    .join(' ');

  const title =
    problems[0] ??
    (open ? `${port.label}（必填，还没接）` : (port.hint ?? `${port.label} · ${portBadge(port.ty)}`));

  return (
    <div className={className} title={title}>
      {direction === 'in' && (
        <Handle
          type="target"
          position={Position.Left}
          id={port.id}
          className="port-dot"
          style={{ background: ink }}
        />
      )}
      <span className="port__chip" style={{ color: ink }}>
        {portBadge(port.ty)}
      </span>
      <span className="port__label">{port.label}</span>
      {direction === 'out' && (
        <Handle
          type="source"
          position={Position.Right}
          id={port.id}
          className="port-dot"
          style={{ background: ink }}
        />
      )}
    </div>
  );
}

/**
 * 拖动的时候 React Flow 每帧都会给节点组件塞新的 `positionAbsoluteX/Y`（还有
 * `dragging`、量出来的宽高）。可卡片本身**不看坐标** —— 移动是外层那层
 * `transform` 干的。所以只比真正会改变画面的三样东西。
 *
 * 少了这个，拖一张卡就要把它整棵子树重建一遍：每个参数控件都是一坨 Radix 组件，
 * 还有预览图和棋盘格背景。
 */
export function sameNodeViewProps(
  before: Readonly<NodeProps<ToolNode>>,
  after: Readonly<NodeProps<ToolNode>>,
): boolean {
  return before.id === after.id && before.data === after.data && before.selected === after.selected;
}

/**
 * 卡片本体。
 *
 * `memo` 挡的是**属性**驱动的重渲染；卡片自己订阅的那些 store 切片（`resolved`、
 * `report`、预览图……）变了照样重渲染，所以状态更新一点没漏。
 */
export const ToolNodeView = memo(ToolNodeCard, sameNodeViewProps);
