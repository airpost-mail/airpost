import React from 'react';
import { localized } from '../../intl/localize';
import * as Actions from '../../data/actions';
import AttachmentStore from '../../data/stores/attachment';
import { Message } from '../../data/models/message';
import { AttachmentItem } from '../../components/attachment-items';
import { Spinner } from '../../components/spinner';

export const AttachmentsArea: React.FunctionComponent<{
  draft: Message;
  // True while files dropped on the composer are still being prepared. They
  // aren't on the draft yet, so a placeholder stands in to show the drop was
  // accepted.
  attaching?: boolean;
}> = (props) => {
  const { files, headerMessageId } = props.draft;

  return (
    <div className="attachments-area">
      {files
        .filter((f) => !f.contentId)
        .map((file) => (
          <AttachmentItem
            key={file.id}
            className="file-upload"
            draggable={false}
            filePath={AttachmentStore.pathForFile(file)}
            displayName={file.filename}
            fileIconName={`file-${file.displayExtension()}.png`}
            onRemoveAttachment={() => Actions.removeAttachment(headerMessageId, file)}
          />
        ))}
      {props.attaching && (
        <div className="attaching-messages">
          <Spinner visible />
          <span>{localized('Attaching…')}</span>
        </div>
      )}
    </div>
  );
};
