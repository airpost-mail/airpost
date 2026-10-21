import _ from 'underscore';
import {
  AndQueryExpression,
  OrQueryExpression,
  InQueryExpression,
  FromQueryExpression,
  ToQueryExpression,
  SubjectQueryExpression,
  GenericQueryExpression,
  TextQueryExpression,
  DateQueryExpression,
  UnreadStatusQueryExpression,
  StarredStatusQueryExpression,
  QueryExpression,
  SearchQueryExpressionVisitor,
} from './ast';
import { Folder } from '../../data/models/folder';

const TOP = 'top';

/* An IMAP SEARCH key, or a nested list of keys and their arguments. */
export type IMAPSearchCriterion = string | IMAPSearchCriterion[];

class IMAPSearchQueryFolderFinderVisitor extends SearchQueryExpressionVisitor<
  typeof TOP | string[]
> {
  visit(root: QueryExpression): 'all' | string[] {
    const result = this.visitAndGetRequiredResult(root);
    if (result === TOP) {
      return 'all';
    }
    return result;
  }

  visitAnd(node: AndQueryExpression) {
    const lhs = this.visitAndGetRequiredResult(node.e1);
    const rhs = this.visitAndGetRequiredResult(node.e2);
    if (lhs === TOP) {
      this._result = rhs;
      return;
    }
    if (rhs === TOP) {
      this._result = lhs;
      return;
    }
    this._result = _.intersection(lhs, rhs);
  }

  visitOr(node: OrQueryExpression) {
    const lhs = this.visitAndGetRequiredResult(node.e1);
    const rhs = this.visitAndGetRequiredResult(node.e2);
    if (lhs === TOP || rhs === TOP) {
      this._result = TOP;
      return;
    }
    this._result = _.union(lhs, rhs);
  }

  visitIn(node: InQueryExpression) {
    this._result = [node.text.token.s];
  }

  visitFrom(/* node */) {
    this._result = TOP;
  }

  visitTo(/* node */) {
    this._result = TOP;
  }

  visitSubject(/* node */) {
    this._result = TOP;
  }

  visitGeneric(/* node */) {
    this._result = TOP;
  }

  visitText() {
    // Only reached for In, which reads its folder name directly.
    this._result = TOP;
  }

  visitUnread(/* node */) {
    this._result = TOP;
  }

  visitStarred(/* node */) {
    this._result = TOP;
  }

  visitHasAttachment(/* node */) {
    this._result = TOP;
  }
}

class IMAPSearchQueryExpressionVisitor extends SearchQueryExpressionVisitor<IMAPSearchCriterion> {
  _folder: Folder;

  constructor(folder: Folder) {
    super();
    this._folder = folder;
  }

  visit(root: QueryExpression): IMAPSearchCriterion[] {
    const result = this.visitAndGetRequiredResult(root);
    // visitAnd always produces a list
    if (root instanceof AndQueryExpression && Array.isArray(result)) {
      return result;
    }
    return [result];
  }

  visitAnd(node: AndQueryExpression) {
    const lhs = this.visitAndGetRequiredResult(node.e1);
    const rhs = this.visitAndGetRequiredResult(node.e2);
    let result: IMAPSearchCriterion[] = [];
    if (node.e1 instanceof AndQueryExpression) {
      result = result.concat(lhs);
    } else {
      result.push(lhs);
    }

    if (node.e2 instanceof AndQueryExpression) {
      result = result.concat(rhs);
    } else {
      result.push(rhs);
    }
    this._result = result;
  }

  visitOr(node: OrQueryExpression) {
    const lhs = this.visitAndGetRequiredResult(node.e1);
    const rhs = this.visitAndGetRequiredResult(node.e2);
    this._result = ['OR', lhs, rhs];
  }

  visitFrom(node: FromQueryExpression) {
    const text = this.visitAndGetRequiredResult(node.text);
    this._result = ['FROM', text];
  }

  visitDate(node: DateQueryExpression) {
    throw new Error(`Function not implemented!: ${node}`);
  }

  visitTo(node: ToQueryExpression) {
    const text = this.visitAndGetRequiredResult(node.text);
    this._result = ['TO', text];
  }

  visitSubject(node: SubjectQueryExpression) {
    const text = this.visitAndGetRequiredResult(node.text);
    this._result = ['SUBJECT', text];
  }

  visitGeneric(node: GenericQueryExpression) {
    const text = this.visitAndGetRequiredResult(node.text);
    this._result = ['TEXT', text];
  }

  visitText(node: TextQueryExpression) {
    this._result = node.token.s;
  }

  visitUnread(node: UnreadStatusQueryExpression) {
    this._result = node.status ? 'UNSEEN' : 'SEEN';
  }

  visitStarred(node: StarredStatusQueryExpression) {
    this._result = node.status ? 'FLAGGED' : 'UNFLAGGED';
  }

  visitIn(node: InQueryExpression) {
    const text = this.visitAndGetRequiredResult(node.text);
    this._result = text === this._folder.name ? 'ALL' : '!ALL';
  }

  visitHasAttachment(/* node */) {
    this._result = [
      'OR',
      ['HEADER', 'Content-Type', 'multipart/mixed'],
      ['HEADER', 'Content-Type', 'multipart/related'],
    ];
  }
}

export default class IMAPSearchQueryBackend {
  static ALL_FOLDERS() {
    return 'all';
  }

  static compile(ast: QueryExpression, folder: Folder) {
    return new IMAPSearchQueryBackend().compile(ast, folder);
  }

  static folderNamesForQuery(ast: QueryExpression) {
    return new IMAPSearchQueryFolderFinderVisitor().visit(ast);
  }

  compile(ast: QueryExpression, folder: Folder) {
    return new IMAPSearchQueryExpressionVisitor(folder).visit(ast);
  }
}
