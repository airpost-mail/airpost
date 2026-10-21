import React, { Component } from 'react';
import { Menu } from '../../components/menu';
import { RetinaImg } from '../../components/retina-img';
import { LabelColorizer } from '../../components/mail-label';
import BoldedSearchResult from '../../components/bolded-search-result';
import * as Utils from '../../data/models/utils';
import { localized } from '../../intl/localize';
import * as Actions from '../../data/actions';
import TaskQueue from '../../data/stores/task-queue';
import { Thread } from '../../data/models/thread';
import { Account } from '../../data/models/account';
import { Category } from '../../data/models/category';
import CategoryStore from '../../data/stores/category';
import { Folder } from '../../data/models/folder';
import { Label } from '../../data/models/label';
import { SyncbackCategoryTask } from '../../data/tasks/syncback-category';
import { ChangeFolderTask } from '../../data/tasks/change-folder';
import { ChangeLabelsTask } from '../../data/tasks/change-labels';
import FocusedPerspectiveStore from '../../data/stores/focused-perspective';
import { Categories } from '../../data/query/observables';
import { TextField } from '../../components/kit/text-field';
import { CategoryData } from './types';

interface MovePickerPopoverProps {
  threads: Thread[];
  account: Account;
}

interface MovePickerPopoverState {
  searchValue: string;
  categoryData: CategoryData[];
}

export default class MovePickerPopover extends Component<
  MovePickerPopoverProps,
  MovePickerPopoverState
> {
  _standardFolders: Category[] = [];
  _userCategories: Category[] = [];
  disposables?: Rx.Disposable[];

  constructor(props: MovePickerPopoverProps) {
    super(props);
    this.state = this._recalculateState(this.props, { searchValue: '' });
  }

  componentDidMount() {
    this._registerObservables();
  }

  componentDidUpdate(prevProps: MovePickerPopoverProps) {
    if (prevProps.account !== this.props.account || prevProps.threads !== this.props.threads) {
      // Re-register observables when account/threads change.
      // The subscription callback (_onCategoriesChanged) will trigger setState
      // with the new categories, so we don't need an explicit setState here.
      this._registerObservables();
    }
  }

  componentWillUnmount() {
    this._unregisterObservables();
  }

  _registerObservables = (props = this.props) => {
    this._unregisterObservables();
    this.disposables = [
      Categories.forAccount(props.account).sort().subscribe(this._onCategoriesChanged),
    ];
  };

  _unregisterObservables = () => {
    if (this.disposables) {
      this.disposables.forEach((disp) => disp.dispose());
    }
  };

  _recalculateState = (props = this.props, { searchValue = this.state.searchValue || '' } = {}) => {
    const { threads, account } = props;
    if (threads.length === 0) {
      return { categoryData: [], searchValue };
    }

    const currentCategories = FocusedPerspectiveStore.current().categories() || [];
    const currentCategoryIds = currentCategories.map((c) => c.id);
    const viewingAllMail = !currentCategories.find((c) => c.role === 'spam' || c.role === 'trash');
    const hidden = account ? ['drafts', 'sent', 'snoozed'] : [];

    if (viewingAllMail) {
      hidden.push('all');
    }

    // Compile the search regex once and reuse it across the .filter below.
    const searchRe = Utils.wordSearchRegExp(searchValue);
    const divider: CategoryData = { divider: true, id: 'category-divider' };
    const categoryData = [...this._standardFolders, divider, ...this._userCategories]
      .filter(
        (cat) =>
          // remove categories that are part of the current perspective or locked
          !(cat instanceof Category) ||
          (!hidden.includes(cat.role) && !currentCategoryIds.includes(cat.id))
      )
      .filter((cat) => searchRe.test(String(cat.displayName)))
      .map((cat): CategoryData => {
        if (!(cat instanceof Category)) {
          return cat;
        }
        return {
          id: cat.id,
          category: cat,
          displayName: cat.displayName,
          backgroundColor: LabelColorizer.backgroundColorDark(cat),
        };
      });

    if (searchValue.length > 0) {
      const newItemData: CategoryData = {
        searchValue: searchValue,
        newCategoryItem: true,
        id: 'category-create-new',
      };
      categoryData.push(newItemData);
    }
    return { categoryData, searchValue };
  };

  _onCategoriesChanged = (categories: Category[]) => {
    this._standardFolders = categories.filter((c) => c.role && c instanceof Folder);
    this._userCategories = categories.filter((c) => !c.role || !(c instanceof Folder));
    // Use functional setState to preserve any pending searchValue updates from user typing
    this.setState((prevState) =>
      this._recalculateState(this.props, { searchValue: prevState.searchValue })
    );
  };

  _onEscape = () => {
    Actions.closePopover();
  };

  _onSelectCategory = (item: CategoryData) => {
    if (this.props.threads.length === 0) {
      return;
    }

    if (item.newCategoryItem) {
      this._onCreateCategory();
    } else {
      this._onMoveToCategory(item);
    }
    Actions.popSheet();
    Actions.closePopover();
  };

  _onCreateCategory = () => {
    const syncbackTask = SyncbackCategoryTask.forCreating({
      name: this.state.searchValue,
      accountId: this.props.account.id,
    });

    TaskQueue.waitForPerformRemote(syncbackTask).then((finishedTask) => {
      if (!finishedTask.created) {
        AppEnv.showErrorDialog({ title: 'Error', message: localized(`Could not create folder.`) });
        return;
      }
      this._onMoveToCategory({ category: finishedTask.created });
    });
    Actions.queueTask(syncbackTask);
  };

  _onMoveToCategory = ({ category }: CategoryData) => {
    const { threads } = this.props;

    if (category instanceof Folder) {
      // Move only the copies shown in the current folder, as remove / swipe / drag do
      // (see CategoryMailboxPerspective.tasksForRemovingItems).
      const currentCat = (FocusedPerspectiveStore.current().categories() || []).find(
        (c) => c.accountId === this.props.account.id
      );
      Actions.queueTask(
        new ChangeFolderTask({
          source: 'Category Picker: New Category',
          threads: threads,
          folder: category,
          sourceFolderIds: currentCat instanceof Folder ? [currentCat.id] : [],
        })
      );
    } else {
      const all: Label[] = [];
      threads.forEach(({ labels }) => all.push(...labels));

      Actions.queueTask(
        new ChangeLabelsTask({
          source: 'Category Picker: New Category',
          labelsToRemove: all,
          labelsToAdd: [category],
          threads: threads,
        })
      );
    }
  };

  _onCompleteAutosuggest = (item: CategoryData) => {
    this.setState(this._recalculateState(this.props, { searchValue: item.displayName }));
  };

  _onSearchValueChange = (event: React.ChangeEvent<HTMLInputElement>) => {
    this.setState(this._recalculateState(this.props, { searchValue: event.target.value }));
  };

  _renderCreateNewItem = ({ searchValue }: CategoryData) => {
    const icon =
      CategoryStore.getInboxCategory(this.props.account) instanceof Folder ? 'folder' : 'tag';

    return (
      <div className="category-item category-create-new">
        <RetinaImg
          name={`${icon}.png`}
          className={`category-create-new-${icon}`}
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

    const icon =
      item.category instanceof Folder ? (
        <RetinaImg
          name={`${item.name}.png`}
          fallback={'folder.png'}
          mode={RetinaImg.Mode.ContentIsMask}
        />
      ) : (
        <RetinaImg name={`tag.png`} mode={RetinaImg.Mode.ContentIsMask} />
      );

    return (
      <div className="category-item">
        {icon}
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
        placeholder={localized('Move to...')}
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
          onSelect={this._onSelectCategory}
          onExpand={this._onCompleteAutosuggest}
          onEscape={this._onEscape}
          defaultSelectedIndex={this.state.searchValue === '' ? -1 : 0}
        />
      </div>
    );
  }
}
