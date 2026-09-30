import type { ComponentPropsWithRef } from 'react';

const base = 'ui-input';

export function Input({ className, ...rest }: ComponentPropsWithRef<'input'>) {
  return <input {...rest} className={[base, className].filter(Boolean).join(' ')} />;
}

export function Textarea({ className, ...rest }: ComponentPropsWithRef<'textarea'>) {
  return <textarea {...rest} className={[base, 'ui-input--area', className].filter(Boolean).join(' ')} />;
}
