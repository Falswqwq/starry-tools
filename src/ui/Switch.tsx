import * as Primitive from '@radix-ui/react-switch';

export function Switch({
  id,
  checked,
  onCheckedChange,
  labelledBy,
}: {
  id?: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  labelledBy?: string;
}) {
  return (
    <span className="ui-switch-row">
      <Primitive.Root
        id={id}
        checked={checked}
        onCheckedChange={onCheckedChange}
        aria-labelledby={labelledBy}
        className="ui-switch nodrag"
      >
        <Primitive.Thumb className="ui-switch__thumb" />
      </Primitive.Root>
      <span className="ui-switch__text" aria-hidden="true">
        {checked ? '开' : '关'}
      </span>
    </span>
  );
}
