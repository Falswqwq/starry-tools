/** 浮动圆角窗口的公共外壳。所有弹层都走这一处，样式才统一。 */

import * as Primitive from '@radix-ui/react-popover';
import { X } from 'lucide-react';
import type { ComponentPropsWithoutRef, ReactNode } from 'react';

import { Button } from './Button';

export const Popover = Primitive.Root;
export const PopoverTrigger = Primitive.Trigger;

type ContentProps = ComponentPropsWithoutRef<typeof Primitive.Content>;

export function PopoverContent({ className, children, ...rest }: ContentProps) {
  return (
    <Primitive.Portal>
      <Primitive.Content
        className={['popover', className].filter(Boolean).join(' ')}
        sideOffset={8}
        collisionPadding={12}
        {...rest}
      >
        {children}
      </Primitive.Content>
    </Primitive.Portal>
  );
}

/** 窗口顶栏：标题 + 收起按钮。 */
export function PopoverHeader({
  title,
  meta,
  children,
}: {
  title: string;
  meta?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <header className="popover__head">
      <h2 className="popover__title">{title}</h2>
      {meta && <span className="popover__meta">{meta}</span>}
      <span className="popover__spacer" />
      {children}
      <Primitive.Close asChild>
        <Button size="icon-sm" variant="ghost" tooltip="收起">
          <X size={13} />
        </Button>
      </Primitive.Close>
    </header>
  );
}
