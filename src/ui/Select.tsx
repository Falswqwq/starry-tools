/** Radix 的下拉选择框。不用原生 `<select>`，弹层样式和键盘操作都自己说了算。 */

import * as Primitive from '@radix-ui/react-select';
import { Check, ChevronDown } from 'lucide-react';

export type SelectOption = {
  value: string;
  label: string;
  hint?: string;
};

export type SelectProps = {
  id?: string;
  value: string;
  options: SelectOption[];
  onValueChange: (value: string) => void;
  placeholder?: string;
  labelledBy?: string;
};

export function Select({
  id,
  value,
  options,
  onValueChange,
  placeholder = '未设置',
  labelledBy,
}: SelectProps) {
  const known = options.some((option) => option.value === value);

  return (
    <Primitive.Root
      value={known ? value : ''}
      onValueChange={onValueChange}
    >
      <Primitive.Trigger id={id} aria-labelledby={labelledBy} className="ui-select nodrag">
        <Primitive.Value placeholder={placeholder} />
        <Primitive.Icon className="ui-select__caret">
          <ChevronDown size={12} strokeWidth={2.5} />
        </Primitive.Icon>
      </Primitive.Trigger>

      <Primitive.Portal>
        <Primitive.Content className="ui-menu" position="popper" sideOffset={4} align="start">
          <Primitive.Viewport className="ui-menu__viewport">
            {options.map((option) => (
              <Primitive.Item key={option.value} value={option.value} className="ui-menu__item">
                {/* 这一格永远渲染，外面那层是占位的。
                    不能直接把 ItemIndicator 当作栅格子项 —— 未选中时它渲染 null，
                    后面的 ItemText 会被挤进第一列（一个很窄的列），文字就竖着排了。 */}
                <span className="ui-menu__check">
                  <Primitive.ItemIndicator>
                    <Check size={11} strokeWidth={3} />
                  </Primitive.ItemIndicator>
                </span>
                <Primitive.ItemText>{option.label}</Primitive.ItemText>
                {option.hint && <span className="ui-menu__hint">{option.hint}</span>}
              </Primitive.Item>
            ))}
          </Primitive.Viewport>
        </Primitive.Content>
      </Primitive.Portal>
    </Primitive.Root>
  );
}
