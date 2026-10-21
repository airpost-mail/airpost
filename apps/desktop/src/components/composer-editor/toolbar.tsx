import React from 'react';
import { Editor, Value } from 'slate';
import { ComposerEditorPlugin } from './types';

interface ComposerEditorToolbarProps {
  editor: Editor;
  value: Value;
  plugins: ComposerEditorPlugin[];
}

export interface ComposerEditorToolbarState {
  visible: boolean;
}

export default class ComposerEditorToolbar extends React.Component<
  ComposerEditorToolbarProps,
  ComposerEditorToolbarState
> {
  _mounted = false;
  _topClip = 0;
  _el: HTMLElement | null = null;
  _floatingEl: HTMLElement | null = null;
  _heightObserver: ResizeObserver | null = null;

  state = { visible: false };

  componentDidMount() {
    this._mounted = true;
    setTimeout(() => {
      if (!this._mounted) return;
      this.setState({ visible: true }, () => {
        if (!this._mounted) return;
        for (const el of Array.from(document.querySelectorAll('.scroll-region-content'))) {
          el.addEventListener('scroll', this._onScroll);
        }

        // Rendered with visible: true, so both refs are set.
        const el = this._el!;
        const floatingEl = this._floatingEl!;
        const parentScrollRegion = el.closest('.scroll-region-content');
        if (parentScrollRegion) {
          this._topClip = parentScrollRegion.getBoundingClientRect().top;
        } else {
          this._topClip = 0;
        }

        el.style.height = `${floatingEl.clientHeight}px`;
        this._heightObserver = new window.ResizeObserver((entries) => {
          el.style.height = `${floatingEl.clientHeight}px`;
          this._onScroll();
        });
        this._heightObserver.observe(floatingEl);
      });
    }, 400);
  }

  componentWillUnmount() {
    this._mounted = false;
    if (this._heightObserver) {
      this._heightObserver.disconnect();
    }
    for (const el of Array.from(document.querySelectorAll('.scroll-region-content'))) {
      el.removeEventListener('scroll', this._onScroll);
    }
  }

  _onScroll = () => {
    const floatingEl = this._floatingEl;
    if (!this._el || !floatingEl) return;
    const { top, height } = this._el.getBoundingClientRect();
    // A mounted toolbar is in the document, so it has a parent.
    const max = this._el.parentElement!.clientHeight - height;

    if (top < this._topClip) {
      floatingEl.style.position = 'absolute';
      floatingEl.style.top = `${Math.min(max, this._topClip - top)}px`;
    } else {
      floatingEl.style.position = 'relative';
      floatingEl.style.top = '0px';
    }
  };

  render() {
    const { editor, plugins, value } = this.props;

    const sectionItems: JSX.Element[] = [];

    if (!this.state.visible) {
      return (
        <div className="RichEditor-toolbar">
          <div className="floating-container">
            <div className="inner display-deferrable deferred" />
          </div>
        </div>
      );
    }

    const pluginsWithToolbars = plugins.flatMap(({ toolbarComponents, toolbarSectionClass }) =>
      toolbarComponents && toolbarComponents.length
        ? [{ toolbarComponents, toolbarSectionClass }]
        : []
    );

    pluginsWithToolbars.forEach(({ toolbarComponents, toolbarSectionClass }, idx) => {
      sectionItems.push(
        ...toolbarComponents.map((Component, cdx) => (
          <Component
            key={`${idx}-${cdx}`}
            editor={editor}
            value={value}
            className={toolbarSectionClass}
          />
        ))
      );
      if (idx < pluginsWithToolbars.length - 1) {
        sectionItems.push(
          <div key={`${idx}-divider`} className={`divider ${toolbarSectionClass}`} />
        );
      }
    });

    return (
      <div ref={(el) => (this._el = el)} className="RichEditor-toolbar">
        <div ref={(el) => (this._floatingEl = el)} className="floating-container">
          <div className="inner display-deferrable">{sectionItems}</div>
        </div>
      </div>
    );
  }
}
