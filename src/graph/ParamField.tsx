/**
 * 节点上的一个参数控件。控件全部来自 `ui/`，后端只负责说「这是什么参数」。
 *
 * 控件都带 `nodrag`（多行文本框还带 `nowheel`）：React Flow 只在节点内部找
 * `nodrag` 这个类，没有它的话，在控件上按下鼠标就会开始拖节点，而下拉弹层会把
 * mouseup 吃掉，节点就粘在鼠标上了。
 */

import { useEffect, useId, useState } from 'react';

import { countRender } from '../lib/dev-render';
import { describeImage } from '../lib/graph';
import type { ImageInfo, ParamDef } from '../lib/types';
import { Button } from '../ui/Button';
import { Input, Textarea } from '../ui/Field';
import { Select } from '../ui/Select';
import { Slider } from '../ui/Slider';
import { Switch } from '../ui/Switch';

type Props = {
  def: ParamDef;
  value: unknown;
  onChange: (value: unknown) => void;
  onPickFile: () => void;
  fileInfo?: ImageInfo;
};

export function ParamField({ def, value, onChange, onPickFile, fileInfo }: Props) {
  countRender('参数控件');
  // 画布上可能有多个同类型的节点，用 useId 保证 label 和控件对得上。
  const fieldId = useId();
  const labelId = `${fieldId}-label`;

  return (
    <div className="param">
      {def.control === 'bool' ? (
        <div className="param__inline">
          <span className="param__label" id={labelId}>
            {def.label}
          </span>
          <Switch
            id={fieldId}
            checked={value === true}
            onCheckedChange={onChange}
            labelledBy={labelId}
          />
        </div>
      ) : (
        <>
          <label className="param__label" id={labelId} htmlFor={fieldId}>
            {def.label}
          </label>
          <div className="param__control">
            <Control
              def={def}
              value={value}
              onChange={onChange}
              onPickFile={onPickFile}
              fileInfo={fileInfo}
              fieldId={fieldId}
              labelId={labelId}
            />
          </div>
        </>
      )}
      {def.description && <p className="param__note">{def.description}</p>}
    </div>
  );
}

function Control({
  def,
  value,
  onChange,
  onPickFile,
  fileInfo,
  fieldId,
  labelId,
}: Props & { fieldId: string; labelId: string }) {
  switch (def.control) {
    case 'number':
      return <NumberField def={def} value={value} onChange={onChange} fieldId={fieldId} />;
    case 'slider':
      return (
        <Slider
          id={fieldId}
          labelledBy={labelId}
          value={typeof value === 'number' ? value : Number(def.default ?? 0)}
          min={def.min ?? 0}
          max={def.max ?? 100}
          step={def.step ?? 1}
          integer={def.integer}
          unit={def.unit}
          onValueChange={onChange}
        />
      );
    case 'text':
      return <TextField def={def} value={value} onChange={onChange} fieldId={fieldId} />;
    case 'select':
      return (
        <Select
          id={fieldId}
          labelledBy={labelId}
          value={typeof value === 'string' ? value : String(def.default ?? '')}
          options={def.options ?? []}
          onValueChange={onChange}
        />
      );
    case 'file':
      return <FileField def={def} value={value} fileInfo={fileInfo} onPickFile={onPickFile} />;
    default:
      return null;
  }
}

const clamp = (def: ParamDef, raw: number) => {
  const min = def.min ?? Number.NEGATIVE_INFINITY;
  const max = def.max ?? Number.POSITIVE_INFINITY;
  const bounded = Math.min(Math.max(raw, min), max);
  return def.integer ? Math.round(bounded) : bounded;
};

const show = (value: unknown, def: ParamDef) => {
  const fallback = typeof def.default === 'number' ? def.default : 0;
  const parsed = typeof value === 'number' ? value : Number(value);
  return Number.isFinite(parsed) ? String(parsed) : String(fallback);
};

function NumberField({
  def,
  value,
  onChange,
  fieldId,
}: Pick<Props, 'def' | 'value' | 'onChange'> & { fieldId: string }) {
  const [draft, setDraft] = useState(() => show(value, def));

  // 只有外部值真的变了（载入工作流、换了节点）才覆盖草稿，打字时不会被拽回去。
  useEffect(() => {
    if (Number(draft) !== Number(value)) setDraft(show(value, def));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value]);

  function commit(raw: string) {
    setDraft(raw);
    if (raw.trim() === '') return;
    const parsed = Number(raw);
    if (Number.isFinite(parsed)) onChange(clamp(def, parsed));
  }

  return (
    <div className="field">
      <Input
        id={fieldId}
        className="ui-input--number nodrag"
        type="text"
        inputMode="decimal"
        value={draft}
        onChange={(event) => commit(event.target.value)}
        onBlur={() => {
          const parsed = Number(draft);
          const next =
            draft.trim() === '' || !Number.isFinite(parsed)
              ? Number(def.default ?? 0)
              : clamp(def, parsed);
          setDraft(String(next));
          onChange(next);
        }}
      />
      {def.unit && <span className="field__unit">{def.unit}</span>}
    </div>
  );
}

function TextField({
  def,
  value,
  onChange,
  fieldId,
}: Pick<Props, 'def' | 'value' | 'onChange'> & { fieldId: string }) {
  const text = typeof value === 'string' ? value : '';
  if (def.multiline) {
    return (
      <Textarea
        id={fieldId}
        className="nodrag nowheel"
        rows={3}
        value={text}
        placeholder={def.placeholder}
        onChange={(event) => onChange(event.target.value)}
      />
    );
  }
  return (
    <Input
      id={fieldId}
      className="nodrag"
      type="text"
      value={text}
      placeholder={def.placeholder}
      onChange={(event) => onChange(event.target.value)}
    />
  );
}

function FileField({
  def,
  value,
  fileInfo,
  onPickFile,
}: Pick<Props, 'def' | 'value' | 'fileInfo' | 'onPickFile'>) {
  const path = typeof value === 'string' ? value : '';
  const name = path ? path.split(/[/\\]/).pop() : '';
  const isDirectory = def.directory === true;
  const empty = isDirectory ? '选择目录…' : '选择文件…';
  const again = isDirectory ? '换一个目录' : '换一个文件';
  const wrapper = ['file-pick', name ? 'is-filled' : ''].filter(Boolean).join(' ');

  return (
    <div className="field field--stack">
      {/* 空着时是个虚线框的按钮；选完之后文件名顶在它原来的位置，
          鼠标移上来文件名退开、重选的按钮又露出来。 */}
      <div className={wrapper} title={path || undefined}>
        <Button
          size="sm"
          variant="ghost"
          className="ui-btn--block file-pick__btn"
          onClick={onPickFile}
        >
          {name ? again : empty}
        </Button>
        {name && <span className="file-pick__label">{name}</span>}
      </div>
      {fileInfo && <p className="file__meta">{describeImage(fileInfo)}</p>}
      {!name && <p className="param__note">{def.dialogTitle}</p>}
    </div>
  );
}
