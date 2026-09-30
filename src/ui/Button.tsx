/**
 * 基础按钮。
 *
 * 图标按钮拿 `tooltip` 当 aria-label，所以它同时也是可访问名称。
 * 提示挂在按钮**里面**（Tooltip.Root → Trigger → button），这样即使外面又套了
 * 一层 Radix 的 `asChild`（比如 Popover.Trigger），ref 和事件仍然能落到真正的
 * `<button>` 上。
 */

import * as Tip from '@radix-ui/react-tooltip';
import type { ComponentPropsWithRef } from 'react';

export type ButtonVariant = 'solid' | 'ghost' | 'primary' | 'danger';
export type ButtonSize = 'md' | 'sm' | 'icon' | 'icon-sm';

export type ButtonProps = ComponentPropsWithRef<'button'> & {
  variant?: ButtonVariant;
  size?: ButtonSize;
  /** 悬停提示。图标按钮会顺便拿它当 aria-label。 */
  tooltip?: string;
  tooltipSide?: 'top' | 'right' | 'bottom' | 'left';
};

export function Button({
  variant = 'solid',
  size = 'md',
  tooltip,
  tooltipSide = 'bottom',
  className,
  children,
  ...rest
}: ButtonProps) {
  const icon = size === 'icon' || size === 'icon-sm';
  const button = (
    <button
      type="button"
      {...rest}
      aria-label={rest['aria-label'] ?? (icon ? tooltip : undefined)}
      // 按钮一律带 nodrag：画布上的按钮按下去不应该拖动节点。
      className={['ui-btn', `ui-btn--${variant}`, `ui-btn--${size}`, 'nodrag', className]
        .filter(Boolean)
        .join(' ')}
    >
      {children}
    </button>
  );

  if (!tooltip) return button;

  return (
    <Tip.Root>
      <Tip.Trigger asChild>{button}</Tip.Trigger>
      <Tip.Portal>
        <Tip.Content className="ui-tooltip" side={tooltipSide} sideOffset={6}>
          {tooltip}
        </Tip.Content>
      </Tip.Portal>
    </Tip.Root>
  );
}
