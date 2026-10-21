import React from 'react';
import { isEqual } from 'underscore';
import { localized } from '../../intl/localize';
import { Button } from '../../components/kit/button';

interface ListEditorProps<T> {
  items: T[];
  itemTemplate: T;
  onChange: (items: T[]) => void;
  children: (item: T, onChange: (item: Partial<T>) => void) => React.ReactNode;
}

export class ListEditor<T> extends React.Component<ListEditorProps<T>> {
  render() {
    const { items, itemTemplate, children, onChange } = this.props;

    const displayed = items.length ? items : [itemTemplate];

    return (
      <div>
        {displayed.map((item, idx) => (
          <div className="list-editor-item" key={idx}>
            {children(item, (changes) => {
              onChange([...items.slice(0, idx), { ...item, ...changes }, ...items.slice(idx + 1)]);
            })}
            {isEqual(item, itemTemplate) ? (
              <>
                <div className="add-spacer" />
                <div className="add-spacer" />
              </>
            ) : (
              <>
                <Button
                  variant="outline"
                  size="sm"
                  className="remove"
                  aria-label={localized('Remove')}
                  onClick={() => {
                    onChange([...items.slice(0, idx), ...items.slice(idx + 1)]);
                  }}
                >
                  –
                </Button>
                {idx === items.length - 1 ? (
                  <Button
                    variant="outline"
                    size="sm"
                    className="add"
                    aria-label={localized('Add')}
                    onClick={() => {
                      onChange([...items, { ...itemTemplate }]);
                    }}
                  >
                    +
                  </Button>
                ) : (
                  <div className="add-spacer" />
                )}
              </>
            )}
          </div>
        ))}
      </div>
    );
  }
}
