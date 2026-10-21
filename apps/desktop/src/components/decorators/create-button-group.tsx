import React from 'react';

export default function CreateButtonGroup<P extends object>(
  name: string,
  buttons: Array<React.ComponentType<P>>,
  { order = 0 }: { order?: number }
) {
  const fn = (props: P) => {
    return (
      <div className="button-group" style={{ order }}>
        {buttons.map((Component) => (
          <Component key={Component.displayName} {...props} />
        ))}
      </div>
    );
  };
  fn.displayName = name;
  return fn;
}
