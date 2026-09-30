/** 工作流信息：名称之外的说明，以及几个概要数字。 */

import { FileText } from 'lucide-react';

import { formatTime } from '../lib/graph';
import { useStore } from '../state/store';
import { Button } from '../ui/Button';
import { Textarea } from '../ui/Field';
import { Popover, PopoverContent, PopoverTrigger } from '../ui/Popover';

export function DescriptionMenu() {
  const meta = useStore((state) => state.meta);
  const nodes = useStore((state) => state.nodes);
  const edges = useStore((state) => state.edges);
  const setMeta = useStore((state) => state.setMeta);

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button size="icon" variant="solid" tooltip="说明与概要">
          <FileText size={14} />
        </Button>
      </PopoverTrigger>

      <PopoverContent className="info" side="bottom" align="start" aria-label="工作流说明">
        <header className="popover__head">
          <h2 className="popover__title">说明</h2>
          <span className="popover__spacer" />
          <span className="popover__meta">更新于 {formatTime(meta.updatedAt)}</span>
        </header>

        <div className="info__body">
          <label className="field-label" htmlFor="wf-desc">
            这个工作流是做什么的
          </label>
          <Textarea
            id="wf-desc"
            rows={4}
            placeholder="例如：把 webp 素材转成 png，再放大到四倍给像素画用…"
            value={meta.description}
            onChange={(event) => setMeta({ description: event.target.value })}
          />

          <dl className="facts">
            <div>
              <dt>节点</dt>
              <dd>{nodes.length}</dd>
            </div>
            <div>
              <dt>连线</dt>
              <dd>{edges.length}</dd>
            </div>
          </dl>
        </div>
      </PopoverContent>
    </Popover>
  );
}
