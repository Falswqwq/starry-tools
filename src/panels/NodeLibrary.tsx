/**
 * 节点库：左侧一个独立浮动按钮 → 靠左的浮动圆角窗口。
 *
 * 左栏是分类，右栏是工具卡片。卡片点一下会向上浮起并展开（注意事项、参数、端口），
 * 直接拖出去则会在指针位置落到画布上，并接一段入场动画。
 *
 * 点击和拖动共用一次按下动作，靠 5px 位移阈值区分：没动就是展开/收起，动了才算拖。
 */

import * as Collapsible from '@radix-ui/react-collapsible';
import * as ToggleGroup from '@radix-ui/react-toggle-group';
import { useReactFlow } from '@xyflow/react';
import { ArrowRight, Blocks, MoveRight } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

import { canvasHost } from '../graph/canvas-host';
import { portBadge, portInk } from '../lib/ports';
import type { NodeKind, ParamDef } from '../lib/types';
import { useStore } from '../state/store';
import { Button } from '../ui/Button';
import { Popover, PopoverContent, PopoverTrigger } from '../ui/Popover';

const ALL = '__all__';
const DRAG_THRESHOLD = 5;
/** 和 CSS 里 `.node` 的宽度保持一致，落点才对得准。 */
const NODE_WIDTH = 216;

type Ghost = { kindId: string; x: number; y: number; leaving: boolean };

export function NodeLibrary() {
  const kinds = useStore((state) => state.kinds);
  const addNode = useStore((state) => state.addNode);
  const { screenToFlowPosition } = useReactFlow();

  const [open, setOpen] = useState(false);
  const [category, setCategory] = useState(ALL);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [ghost, setGhost] = useState<Ghost | null>(null);
  const suppressClick = useRef(false);

  const categories = useMemo(() => {
    const buckets = new Map<string, number>();
    for (const kind of kinds) buckets.set(kind.category, (buckets.get(kind.category) ?? 0) + 1);
    return [
      { value: ALL, label: '全部', count: kinds.length },
      ...[...buckets].map(([label, count]) => ({ value: label, label, count })),
    ];
  }, [kinds]);

  const visible = category === ALL ? kinds : kinds.filter((kind) => kind.category === category);

  function place(kindId: string, clientX: number, clientY: number) {
    const point = screenToFlowPosition({ x: clientX, y: clientY });
    // 让节点大致落在指针正下方，而不是左上角顶着指针。
    addNode(kindId, { x: point.x - NODE_WIDTH / 2, y: point.y - 16 });
    setOpen(false);
    setExpanded(null);
  }

  function placeAtCentre(kindId: string) {
    const rect = canvasHost.element?.getBoundingClientRect();
    if (!rect) return;
    place(kindId, rect.left + rect.width / 2, rect.top + rect.height / 2);
  }

  /** 按下卡片：先看是「点」还是「拖」。 */
  function onCardPointerDown(event: React.PointerEvent, kindId: string) {
    if (event.button !== 0) return;
    const startX = event.clientX;
    const startY = event.clientY;
    let dragged = false;

    const onMove = (moveEvent: PointerEvent) => {
      if (!dragged) {
        if (Math.hypot(moveEvent.clientX - startX, moveEvent.clientY - startY) < DRAG_THRESHOLD) {
          return;
        }
        dragged = true;
        suppressClick.current = true;
      }
      setGhost({ kindId, x: moveEvent.clientX, y: moveEvent.clientY, leaving: false });
    };

    const onUp = (upEvent: PointerEvent) => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onCancel);
      window.removeEventListener('keydown', onKeyDown);
      setGhost(null);
      if (!dragged) return;

      // 松开的位置先在原位停一下再淡出，看起来像被放进了画布。
      // 淡出给到 240ms：甩得快的时候 160ms 一眨眼就过去了，根本看不清那张替身。
      setGhost({ kindId, x: upEvent.clientX, y: upEvent.clientY, leaving: true });
      window.setTimeout(() => setGhost(null), 260);
      place(kindId, upEvent.clientX, upEvent.clientY);

      // 松手在卡片上时浏览器还会补一个 click，那一下不该被当成展开/收起。
      // click 在同一个任务里紧跟 pointerup，所以一个 0ms 的定时器足够把它让过去；
      // 如果没补 click，这个定时器也负责把标记清干净，不会误伤下一次点击。
      window.setTimeout(() => {
        suppressClick.current = false;
      }, 0);
    };

    const onCancel = () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onCancel);
      window.removeEventListener('keydown', onKeyDown);
      setGhost(null);
      if (dragged) suppressClick.current = false;
    };

    const onKeyDown = (keyEvent: KeyboardEvent) => {
      if (keyEvent.key === 'Escape') {
        dragged = false;
        onCancel();
      }
    };

    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', onCancel);
    window.addEventListener('keydown', onKeyDown);
  }

  const ghostKind = ghost ? kinds.find((kind) => kind.id === ghost.kindId) : undefined;
  // 正在被拖出去的那张卡片：它自己会被一个向外箭头盖住。
  const draggingKindId = ghost && !ghost.leaving ? ghost.kindId : null;

  return (
    <>
      <Popover
        open={open}
        onOpenChange={(next) => {
          setOpen(next);
          if (!next) setExpanded(null);
        }}
      >
        <PopoverTrigger asChild>
          <Button variant="solid">
            <Blocks size={13} />
            节点库
          </Button>
        </PopoverTrigger>

        <PopoverContent className="library" side="bottom" align="start">
          <header className="popover__head">
            <h2 className="popover__title">节点库</h2>
            <span className="popover__meta">{kinds.length} 个</span>
            <span className="popover__spacer" />
            <span className="popover__hint">拖到画布上放置</span>
          </header>

          <div className="library__body">
            <ToggleGroup.Root
              type="single"
              orientation="vertical"
              className="rail"
              value={category}
              onValueChange={(next) => {
                if (!next) return;
                setCategory(next);
                setExpanded(null);
              }}
            >
              {categories.map((item) => (
                <ToggleGroup.Item key={item.value} value={item.value} className="rail__item">
                  <span className="rail__name">{item.label}</span>
                  <span className="rail__count">{item.count}</span>
                </ToggleGroup.Item>
              ))}
            </ToggleGroup.Root>

            <div
              className="library__list"
              onClick={(event) => {
                // 点在卡与卡之间的空白处就收回展开的卡片。
                if (event.target === event.currentTarget) setExpanded(null);
              }}
            >
              {visible.map((kind) => (
                <ToolCard
                  key={kind.id}
                  kind={kind}
                  open={expanded === kind.id}
                  dragging={draggingKindId === kind.id}
                  onOpenChange={(next) => setExpanded(next ? kind.id : null)}
                  onPointerDown={onCardPointerDown}
                  onPlaceAtCentre={placeAtCentre}
                  suppressClick={suppressClick}
                />
              ))}
            </div>
          </div>
        </PopoverContent>
      </Popover>

      {ghost && ghostKind &&
        createPortal(
          <DragGhostView kind={ghostKind} x={ghost.x} y={ghost.y} leaving={ghost.leaving} />,
          document.body,
        )}
    </>
  );
}

/**
 * 吸附在指针上的那枚胶囊：显示正在搬运的节点名和它的输入输出类型。
 *
 * 「向外」的箭头不在这里 —— 它盖在节点库里被拖走的那张卡片上（见 `ToolCard`）。
 * 拆成独立组件是为了能直接渲染、直接断言：这块的毛病（类名对不上、内容画错）
 * 在真机上只是一闪而过。
 */
export function DragGhostView({
  kind,
  x,
  y,
  leaving = false,
}: {
  kind: NodeKind;
  x: number;
  y: number;
  leaving?: boolean;
}) {
  return (
    <div
      className={`drag-ghost ${leaving ? 'is-leaving' : ''}`}
      style={{ left: x, top: y }}
    >
      <span className="drag-ghost__name">{kind.name}</span>
      <span className="drag-ghost__types">
        <TypeBadges kind={kind} />
      </span>
    </div>
  );
}

export function ToolCard({
  kind,
  open,
  dragging,
  onOpenChange,
  onPointerDown,
  onPlaceAtCentre,
  suppressClick,
}: {
  kind: NodeKind;
  open: boolean;
  /** 这张卡正被拖向画布：内容退开，一个向外箭头盖在上面。 */
  dragging: boolean;
  onOpenChange: (open: boolean) => void;
  onPointerDown: (event: React.PointerEvent, kindId: string) => void;
  onPlaceAtCentre: (kindId: string) => void;
  suppressClick: React.RefObject<boolean>;
}) {
  const root = useRef<HTMLDivElement>(null);

  // 展开之后把它滚进视野 —— 卡多或者上面已经展开着一张的时候尤其需要。
  useEffect(() => {
    if (!open) return;
    const timer = window.setTimeout(() => {
      root.current?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    }, 80);
    return () => window.clearTimeout(timer);
  }, [open]);

  return (
    <Collapsible.Root open={open} onOpenChange={onOpenChange}>
      <div className={`card ${open ? 'is-open' : ''} ${dragging ? 'is-dragging-out' : ''}`} ref={root}>
        <Collapsible.Trigger asChild>
          <button
            type="button"
            className="card__summary"
            onPointerDown={(event) => onPointerDown(event, kind.id)}
            onClick={(event) => {
              // 刚刚是拖动的话，这一下不应该被当成点击。
              if (suppressClick.current) {
                suppressClick.current = false;
                event.preventDefault();
              }
            }}
          >
            <span className="card__top">
              <span className="card__name">{kind.name}</span>
              <span className="card__types">
                <TypeBadges kind={kind} />
              </span>
            </span>
            <span className="card__desc">{kind.description}</span>
            {dragging && (
              <span className="card__out" aria-hidden="true">
                <ArrowRight size={26} strokeWidth={2.8} />
              </span>
            )}
          </button>
        </Collapsible.Trigger>

        <Collapsible.Content className="card__details">
          <div className="card__details-inner">
            {kind.notes.length > 0 && (
              <section className="card__section">
                <h4 className="card__label">注意事项</h4>
                <ul className="card__notes">
                  {kind.notes.map((note) => (
                    <li key={note}>{note}</li>
                  ))}
                </ul>
              </section>
            )}

            {kind.params.length > 0 && (
              <section className="card__section">
                <h4 className="card__label">参数</h4>
                <dl className="card__params">
                  {kind.params.map((def) => (
                    <div className="card__param" key={def.id}>
                      <dt>
                        {def.label}
                        <span className="card__param-meta">{describeParam(def)}</span>
                      </dt>
                      {def.description && <dd>{def.description}</dd>}
                      {def.options && (
                        <dd className="card__param-options">
                          {def.options
                            .map((option) =>
                              option.hint ? `${option.label}（${option.hint}）` : option.label,
                            )
                            .join(' · ')}
                        </dd>
                      )}
                    </div>
                  ))}
                </dl>
              </section>
            )}

            <section className="card__section">
              <h4 className="card__label">端口</h4>
              <ul className="card__io">
                {kind.inputs.map((port) => (
                  <li key={port.id}>
                    <span className="tbadge" style={{ color: portInk(port.ty) }}>
                      {portBadge(port.ty)}
                    </span>
                    <span className="card__io-label">输入 · {port.label}</span>
                    {port.required && <span className="card__io-flag">必填</span>}
                    {port.hint && <span className="card__io-hint">{port.hint}</span>}
                  </li>
                ))}
                {kind.outputs.map((port) => (
                  <li key={port.id}>
                    <span className="tbadge" style={{ color: portInk(port.ty) }}>
                      {portBadge(port.ty)}
                    </span>
                    <span className="card__io-label">输出 · {port.label}</span>
                    {port.hint && <span className="card__io-hint">{port.hint}</span>}
                  </li>
                ))}
              </ul>
            </section>

            <div className="card__cta">
              <Button size="sm" variant="ghost" onClick={() => onPlaceAtCentre(kind.id)}>
                <MoveRight size={11} />
                放到画布中央
              </Button>
              <span className="card__drag-hint">也可以按住卡片拖到画布上</span>
            </div>
          </div>
        </Collapsible.Content>
      </div>
    </Collapsible.Root>
  );
}

function TypeBadges({ kind }: { kind: NodeKind }) {
  return (
    <>
      {kind.inputs.length === 0 ? (
        <span className="tbadge tbadge--start">起点</span>
      ) : (
        kind.inputs.map((port) => (
          <span className="tbadge" style={{ color: portInk(port.ty) }} key={port.id}>
            {portBadge(port.ty)}
          </span>
        ))
      )}
      <ArrowRight size={10} className="card__arrow" aria-hidden="true" />
      {kind.outputs.map((port) => (
        <span className="tbadge" style={{ color: portInk(port.ty) }} key={port.id}>
          {portBadge(port.ty)}
        </span>
      ))}
    </>
  );
}

/** 参数一行摘要：控件种类 + 取值范围 + 默认值。 */
function describeParam(def: ParamDef): string {
  const parts: string[] = [];
  switch (def.control) {
    case 'number':
    case 'slider':
      parts.push(def.control === 'slider' ? '滑杆' : '数字');
      parts.push(
        `${formatNumber(def.min ?? 0)}–${formatNumber(def.max ?? 0)}${def.unit ? ` ${def.unit}` : ''}`,
      );
      parts.push(`默认 ${formatNumber(Number(def.default ?? 0))}`);
      break;
    case 'select': {
      parts.push('下拉');
      const current = def.options?.find((option) => option.value === def.default);
      parts.push(`默认 ${current?.label ?? def.default}`);
      break;
    }
    case 'text':
      parts.push(def.multiline ? '多行文本' : '文本');
      break;
    case 'bool':
      parts.push('开关');
      parts.push(`默认 ${def.default ? '开' : '关'}`);
      break;
    case 'file':
      if (def.directory) {
        parts.push('目录');
        break;
      }
      parts.push('文件');
      parts.push(`${def.extensions?.length ?? 0} 种格式`);
      break;
    default:
      break;
  }
  return parts.join(' · ');
}

function formatNumber(value: number): string {
  if (Number.isInteger(value)) return String(value);
  return String(Math.round(value * 100) / 100);
}
