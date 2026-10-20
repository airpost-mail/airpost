import React from 'react';
import { AccountStore, localized } from 'mailspring-exports';
import { RetinaImg } from 'mailspring-component-kit';
import { IconButton } from '../../../components/kit/icon-button';
import * as OnboardingActions from '../actions';

const PageTopBar = (props: { pageDepth: number; allowMoveBack?: boolean }) => {
  const { pageDepth } = props;

  const closeClass = pageDepth > 1 ? 'back' : 'close';
  const closeIcon = pageDepth > 1 ? 'onboarding-back.png' : 'onboarding-close.png';
  const closeAction = () => {
    const webview = document.querySelector('webview') as Electron.WebviewTag;
    if (webview && webview.canGoBack()) {
      webview.goBack();
    } else if (pageDepth > 1) {
      OnboardingActions.moveToPreviousPage();
    } else {
      if (AccountStore.accounts().length === 0) {
        AppEnv.quit();
      } else {
        AppEnv.close();
      }
    }
  };

  let backButton = (
    <IconButton
      className={closeClass}
      icon={closeIcon}
      mode={RetinaImg.Mode.ContentPreserve}
      label={pageDepth > 1 ? localized('Back') : localized('Close')}
      onClick={closeAction}
    />
  );
  if (props.pageDepth > 1 && !props.allowMoveBack) {
    backButton = null;
  }

  const style: any = {
    top: 0,
    left: 26,
    right: 0,
    height: 27,
    zIndex: 100,
    position: 'absolute',
    WebkitAppRegion: 'drag',
  };
  return (
    <div className="dragRegion" style={style}>
      {backButton}
    </div>
  );
};

export default PageTopBar;
