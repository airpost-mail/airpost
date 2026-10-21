import React, { Component } from 'react';
import DateUtils from '../../utils/date';
import { Message } from '../../data/models/message';
import { Flexbox } from '../../components/flexbox';
import SendingProgressBar from './sending-progress-bar';

export default class DraftListSendStatus extends Component<{ draft: Message }> {
  static displayName = 'DraftListSendStatus';

  static containerRequired = false;

  render() {
    const { draft } = this.props;
    // TODO BG
    // if (draft.uploadTaskId) {
    //   return (
    //     <Flexbox style={{ width: 150, whiteSpace: 'nowrap' }}>
    //       <SendingProgressBar
    //         style={{ flex: 1, marginRight: 10 }}
    //         progress={draft.uploadProgress * 100}
    //       />
    //     </Flexbox>
    //   );
    // }
    return <span className="timestamp">{DateUtils.shortTimeString(draft.date)}</span>;
  }
}
