import * as Primitive from '@radix-ui/react-slider';

/**
 * 一根滑杆，右边跟一个当前值的读数。
 *
 * 行为和可访问性交给 Radix，外观按 `styles/app.css` 里的 token 来。
 * `nodrag` / `nowheel` 不能少：节点里的控件得自己挡掉 React Flow 的拖拽和滚轮缩放。
 */
export function Slider({
  id,
  value,
  min,
  max,
  step,
  integer,
  unit,
  labelledBy,
  onValueChange,
}: {
  id?: string;
  value: number;
  min: number;
  max: number;
  step: number;
  integer?: boolean;
  unit?: string;
  labelledBy?: string;
  onValueChange: (value: number) => void;
}) {
  const shown = integer ? Math.round(value) : Math.round(value * 100) / 100;
  return (
    <span className="ui-slider-row">
      <Primitive.Root
        id={id}
        className="ui-slider nodrag nowheel"
        min={min}
        max={max}
        step={step}
        value={[value]}
        aria-labelledby={labelledBy}
        onValueChange={(next) => {
          const [first] = next;
          if (first !== undefined) onValueChange(first);
        }}
      >
        <Primitive.Track className="ui-slider__track">
          <Primitive.Range className="ui-slider__range" />
        </Primitive.Track>
        <Primitive.Thumb className="ui-slider__thumb" />
      </Primitive.Root>
      <span className="ui-slider__value">
        {shown}
        {unit ?? ''}
      </span>
    </span>
  );
}
