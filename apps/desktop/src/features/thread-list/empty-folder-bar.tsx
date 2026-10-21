import React from 'react';
import ListensToFluxStore from '../../components/decorators/listens-to-flux-store';
import { RetinaImg } from '../../components/retina-img';
import { localized } from '../../intl/localize';
import * as Actions from '../../data/actions';
import { Folder } from '../../data/models/folder';
import TaskQueue from '../../data/stores/task-queue';
import { ExpungeAllInFolderTask } from '../../data/tasks/expunge-all-in-folder';
import FocusedPerspectiveStore from '../../data/stores/focused-perspective';
import ThreadCountsStore from '../../data/stores/thread-counts';
import { Button } from '../../components/kit/button';

interface ThreadListEmptyFolderBarProps {
  role: string;
  folders: Folder[];
  count: number;
  busy: boolean;
}

class ThreadListEmptyFolderBar extends React.Component<ThreadListEmptyFolderBarProps> {
  static displayName = 'ThreadListEmptyFolderBar';

  _onClick = () => {
    const { folders } = this.props;

    Actions.queueTasks(
      folders.map(
        (folder) =>
          new ExpungeAllInFolderTask({
            accountId: folder.accountId,
            folder,
          })
      )
    );
  };

  render() {
    const { role, count, busy } = this.props;
    if (!role || count === 0) {
      return false;
    }
    const term = role === 'trash' ? localized('Deleted').toLocaleLowerCase() : role;

    return (
      <div className="thread-list-empty-folder-bar">
        <div className="notice">
          {count > 1
            ? localized(`Showing %@ threads with %@ messages`, (count / 1).toLocaleString(), term)
            : localized(`Showing 1 thread with %@ messages`, term)}
        </div>
        {busy ? (
          <Button variant="outline" disabled>
            <RetinaImg
              style={{ width: 16, height: 16 }}
              name="inline-loading-spinner.gif"
              mode={RetinaImg.Mode.ContentPreserve}
            />
          </Button>
        ) : (
          <Button variant="outline" onClick={this._onClick}>
            {localized(`Empty %@ now`, role)}
          </Button>
        )}
      </div>
    );
  }
}

export default ListensToFluxStore(ThreadListEmptyFolderBar, {
  stores: [TaskQueue, ThreadCountsStore, FocusedPerspectiveStore],
  getStateFromStores: () => {
    const p = FocusedPerspectiveStore.current();
    const folders = (p && p.categories()) || [];

    if (
      !folders.length ||
      !folders.every((c) => c instanceof Folder && (c.role === 'trash' || c.role === 'spam'))
    ) {
      return { role: null, folders: null };
    }

    return {
      folders,
      role: folders[0].role,
      busy: TaskQueue.findTasks(ExpungeAllInFolderTask).some((t) =>
        folders.map((f) => f.accountId).includes(t.accountId)
      ),
      count: folders.reduce(
        (sum, { id }) => sum + (ThreadCountsStore.totalCountForCategoryId(id) ?? 0),
        0
      ),
    };
  },
});
