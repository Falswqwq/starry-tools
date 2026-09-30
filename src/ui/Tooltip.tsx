import * as Primitive from '@radix-ui/react-tooltip';
import type { ReactNode } from 'react';

export function TooltipProvider({ children }: { children: ReactNode }) {
  return (
    <Primitive.Provider delayDuration={300} skipDelayDuration={120}>
      {children}
    </Primitive.Provider>
  );
}

/**
 * 给单个元素套一个悬停提示。
 *
 * 注意：它会把 props 转交给自己包的那个元素，所以不要拿它去套一层已经用
 * `asChild` 的 Radix Trigger —— 那种情况用 `Button` 的 `tooltip` 属性。
 */
export function Tooltip({
  label,
  side = 'top',
  children,
}: {
  label: string;
  side?: 'top' | 'right' | 'bottom' | 'left';
  children: ReactNode;
}) {
  return (
    <Primitive.Root>
      <Primitive.Trigger asChild>{children as never}</Primitive.Trigger>
      <Primitive.Portal>
        <Primitive.Content className="ui-tooltip" side={side} sideOffset={6}>
          {label}
        </Primitive.Content>
      </Primitive.Portal>
    </Primitive.Root>
  );
}
