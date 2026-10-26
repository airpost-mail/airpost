import React, { Component, CSSProperties } from 'react';
import { Menu } from '../../components/menu';
import { RetinaImg } from '../../components/retina-img';
import { LabelColorizer } from '../../components/mail-label';
import BoldedSearchResult from '../../components/bolded-search-result';
import * as Utils from '../../data/models/utils';
import { localized } from '../../intl/localize';
import * as Actions from '../../data/actions';
import { Label } from '../../data/models/label';
import { Account } from '../../data/models/account';
import { Category } from '../../data/models/category';
import { IntentTask, describeLabel } from '../../data/tasks/intent';
import { Method } from '../../engine/protocol';
import { createCategory } from '../../engine/client/intents';
import { Thread } from '../../data/models/thread';
import { Categories } from '../../data/query/observables';
import { TextField } from '../../components/kit/text-field';
import { CategoryData } from './types';

interface LabelPickerPopoverProps {
  threads: Thread[];
  account: Account;
}

interface LabelPickerPopoverState {
  searchValue: string;
  categoryData: CategoryData[];
}

export default class LabelPickerPopover extends Component<
  LabelPickerPopoverProps,
  LabelPickerPopoverState
> {
  _labels: Label[] = [];
  disposables?: Rx.Disposable[];

  constructor(props: LabelPickerPopoverProps) {
    super(props);
    this.state = this._recalculateState(this.props, { searchValue: '' });
  }

  componentDidMount() {
    this._registerObservables();
  }

  componentDidUpdate(prevProps: LabelPickerPopoverProps) {
    if (prevProps.account !== this.props.account || prevProps.threads !== this.props.threads) {
      // Re-register observables when account/threads change.
      // The subscription callback (_onLabelsChanged) will trigger setState
      // with the new labels, so we don't need an explicit setState here.
      this._registerObservables();
    }
  }

  componentWillUnmount() {
    this._unregisterObservables();
  }

  _registerObservables = (props = this.props) => {
    this._unregisterObservables();
    this.disposables = [
      Categories.forAccount(props.account).sort().subscribe(this._onLabelsChanged),
    ];
  };

  _unregisterObservables = () => {
    if (this.disposables) {
      this.disposables.forEach((disp) => disp.dispose());
    }
  };

  _recalculateState = (props = this.props, { searchValue = this.state.searchValue || '' } = {}) => {
    const { threads } = props;

    if (threads.length === 0) {
      return { categoryData: [], searchValue };
    }

    // Compile the search regex once and reuse it across the .filter below.
    const searchRe = Utils.wordSearchRegExp(searchValue);
    const categoryData = this._labels
      .filter((label) => searchRe.test(label.displayName))
      .map<CategoryData>((label) => {
        return {
          id: label.id,
          category: label,
          displayName: label.displayName,
          backgroundColor: LabelColorizer.backgroundColorDark(label),
          usage: threads.filter((t) => t.categories.find((c) => c.id === label.id)).length,
          numThreads: threads.length,
        };
      });

    if (searchValue.length > 0) {
      categoryData.push({
        searchValue: searchValue,
        newCategoryItem: true,
        id: 'category-create-new',
      });
    }
    return { categoryData, searchValue };
  };

  _onLabelsChanged = (categories: Category[]) => {
    this._labels = categories.filter((c) => {
      return c instanceof Label && !c.role;
    }) as Label[];
    // Use functional setState to preserve any pending searchValue updates from user typing
    this.setState((prevState) =>
      this._recalculateState(this.props, { searchValue: prevState.searchValue })
    );
  };

  _onEscape = () => {
    Actions.closePopover();
  };

  _onSelectLabel = (item: CategoryData) => {
    const { account, threads } = this.props;

    if (threads.length === 0) return;

    const label = (id: string, name: string, add: boolean, source: string) =>
      Actions.queueTask(
        IntentTask.forThreads(
          Method.ThreadsLabel,
          threads,
          add ? { add: [id] } : { remove: [id] },
          source,
          describeLabel(threads.length, name, add)
        )
      );

    if (item.newCategoryItem) {
      const name = this.state.searchValue;
      createCategory(account.id, name).then(
        (created) => label(created.id, name, true, 'Category Picker: New Category'),
        () => AppEnv.showErrorDialog({ title: 'Error', message: `Could not create label.` })
      );
    } else if (item.category) {
      const { id, displayName } = item.category;
      const add = item.usage !== threads.length;
      label(id, displayName, add, 'Category Picker: Existing Category');
    }
    Actions.closePopover();
  };

  _onSearchValueChange = (event: React.ChangeEvent<HTMLInputElement>) => {
    this.setState(this._recalculateState(this.props, { searchValue: event.target.value }));
  };

  _renderCheckbox = (item: CategoryData) => {
    const styles: CSSProperties = {};
    let checkStatus;
    styles.backgroundColor = item.backgroundColor;

    if (item.usage === 0) {
      checkStatus = <span />;
      // Only label rows are rendered with a checkbox, and they carry both counts.
    } else if (item.usage! < item.numThreads!) {
      checkStatus = (
        <RetinaImg
          className="check-img dash"
          name="tagging-conflicted.png"
          mode={RetinaImg.Mode.ContentPreserve}
          onClick={() => this._onSelectLabel(item)}
        />
      );
    } else {
      checkStatus = (
        <RetinaImg
          className="check-img check"
          name="tagging-checkmark.png"
          mode={RetinaImg.Mode.ContentPreserve}
          onClick={() => this._onSelectLabel(item)}
        />
      );
    }

    return (
      <div className="check-wrap" style={styles}>
        <RetinaImg
          className="check-img check"
          name="tagging-checkbox.png"
          mode={RetinaImg.Mode.ContentPreserve}
          onClick={() => this._onSelectLabel(item)}
        />
        {checkStatus}
      </div>
    );
  };

  _renderCreateNewItem = ({ searchValue }: CategoryData) => {
    return (
      <div className="category-item category-create-new">
        <RetinaImg
          name={`tag.png`}
          className={`category-create-new-tag`}
          mode={RetinaImg.Mode.ContentIsMask}
        />
        <div className="category-display-name">
          <strong>&ldquo;{searchValue}&rdquo;</strong> ({localized(`Create`).toLocaleLowerCase()})
        </div>
      </div>
    );
  };

  _renderItem = (item: CategoryData) => {
    if (item.divider) {
      return <Menu.Item key={item.id} divider={item.divider} />;
    } else if (item.newCategoryItem) {
      return this._renderCreateNewItem(item);
    }

    return (
      <div className="category-item">
        {this._renderCheckbox(item)}
        <div className="category-display-name">
          <BoldedSearchResult value={item.displayName} query={this.state.searchValue || ''} />
        </div>
      </div>
    );
  };

  render() {
    const headerComponents = [
      <TextField
        autoFocus
        key="textfield"
        className="search"
        placeholder={localized('Label as...')}
        value={this.state.searchValue}
        onChange={this._onSearchValueChange}
      />,
    ];

    return (
      <div className="category-picker-popover">
        <Menu
          headerComponents={headerComponents}
          footerComponents={[]}
          items={this.state.categoryData}
          itemKey={(item) => item.id}
          itemContent={this._renderItem}
          onSelect={this._onSelectLabel}
          onEscape={this._onEscape}
          defaultSelectedIndex={this.state.searchValue === '' ? -1 : 0}
        />
      </div>
    );
  }
}
