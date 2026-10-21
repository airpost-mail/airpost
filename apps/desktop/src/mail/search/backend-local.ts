import {
  SearchQueryExpressionVisitor,
  FromQueryExpression,
  ToQueryExpression,
  SubjectQueryExpression,
  GenericQueryExpression,
  TextQueryExpression,
  InQueryExpression,
  OrQueryExpression,
  NotQueryExpression,
  AndQueryExpression,
  DateQueryExpression,
  UnreadStatusQueryExpression,
  StarredStatusQueryExpression,
  HasAttachmentQueryExpression,
  MatchQueryExpression,
  QueryExpression,
} from './ast';
import DateUtils from '../../utils/date';

/*
 * This class visits a match-compatible subtree and condenses it into a single
 * MatchQueryExpression.
 */
class MatchQueryExpressionVisitor extends SearchQueryExpressionVisitor<string> {
  visit(root: QueryExpression) {
    const result = this.visitAndGetResult(root);
    return new MatchQueryExpression(`${result}`);
  }

  _assertIsMatchCompatible(node: QueryExpression) {
    if (!node.isMatchCompatible()) {
      throw new Error(`Expected ${node} to be match compatible`);
    }
  }

  visitAnd(node: AndQueryExpression) {
    this._assertIsMatchCompatible(node);
    const lhs = this.visitAndGetResult(node.e1);
    const rhs = this.visitAndGetResult(node.e2);
    this._result = `(${lhs} AND ${rhs})`;
  }

  visitNot(node: NotQueryExpression) {
    this._assertIsMatchCompatible(node);
    const lhs = this.visitAndGetResult(node.e1);
    const rhs = this.visitAndGetResult(node.e2);
    this._result = `(${lhs} NOT ${rhs})`;
  }

  visitOr(node: OrQueryExpression) {
    const lhs = this.visitAndGetResult(node.e1);
    const rhs = this.visitAndGetResult(node.e2);
    this._result = `(${lhs} OR ${rhs})`;
  }

  visitDate(node: DateQueryExpression) {}

  visitFrom(node: FromQueryExpression) {
    const text = this.visitAndGetResult(node.text);
    this._result = `(from_ : "${text}"*)`;
  }

  visitTo(node: ToQueryExpression) {
    const text = this.visitAndGetResult(node.text);
    this._result = `(to_ : "${text}"*)`;
  }

  visitSubject(node: SubjectQueryExpression) {
    const text = this.visitAndGetResult(node.text);
    this._result = `(subject : "${text}"*)`;
  }

  visitGeneric(node: GenericQueryExpression) {
    const text = this.visitAndGetResult(node.text);
    this._result = `("${text}"*)`;
  }

  visitText(node: TextQueryExpression) {
    // Escape double quotes for FTS5 quoted string context. The callers
    // (visitFrom, visitTo, visitSubject, visitGeneric, visitIn) all wrap
    // this text in double quotes, so the only character that needs escaping
    // is the double quote itself (doubled per FTS5 spec). All other special
    // FTS5 characters are neutralized by being inside the quoted phrase.
    this._result = node.token.s.replace(/"/g, '""');
  }

  visitUnread(node: UnreadStatusQueryExpression) {
    this._assertIsMatchCompatible(node);
  }

  visitStarred(node: StarredStatusQueryExpression) {
    this._assertIsMatchCompatible(node);
  }

  visitIn(node: InQueryExpression) {
    const text = this.visitAndGetResult(node.text);
    this._result = `(categories : "${text}"*)`;
  }

  visitHasAttachment(node: HasAttachmentQueryExpression) {
    this._assertIsMatchCompatible(node);
  }
}

/*
 * This class creates a new AST by converting match-compatible subtrees into
 * MatchQueryExpressions.
 */
class MatchCompatibleQueryCondenser extends SearchQueryExpressionVisitor<QueryExpression> {
  private _matchVisitor = new MatchQueryExpressionVisitor();

  visit(root: QueryExpression) {
    return this.visitAndGetRequiredResult(root);
  }

  visitAnd(node: AndQueryExpression) {
    if (node.isMatchCompatible()) {
      this._result = this._matchVisitor.visit(node);
      return;
    }

    const lhs = this.visitAndGetRequiredResult(node.e1);
    const rhs = this.visitAndGetRequiredResult(node.e2);
    this._result = new AndQueryExpression(lhs, rhs);
  }

  visitOr(node: OrQueryExpression) {
    if (node.isMatchCompatible()) {
      this._result = this._matchVisitor.visit(node);
      return;
    }

    const lhs = this.visitAndGetRequiredResult(node.e1);
    const rhs = this.visitAndGetRequiredResult(node.e2);
    this._result = new OrQueryExpression(lhs, rhs);
  }

  visitNot(node: NotQueryExpression) {
    if (node.isMatchCompatible()) {
      this._result = this._matchVisitor.visit(node);
      return;
    }
    const lhs = this.visitAndGetRequiredResult(node.e1);
    const rhs = this.visitAndGetRequiredResult(node.e2);
    this._result = new NotQueryExpression(lhs, rhs);
  }

  visitFrom(node: FromQueryExpression) {
    this._result = this._matchVisitor.visit(node);
  }

  visitTo(node: ToQueryExpression) {
    this._result = this._matchVisitor.visit(node);
  }

  visitSubject(node: SubjectQueryExpression) {
    this._result = this._matchVisitor.visit(node);
  }

  visitGeneric(node: GenericQueryExpression) {
    this._result = this._matchVisitor.visit(node);
  }

  visitText(node: TextQueryExpression) {
    this._result = this._matchVisitor.visit(node);
  }

  visitIn(node: InQueryExpression) {
    this._result = this._matchVisitor.visit(node);
  }

  visitUnread(node: UnreadStatusQueryExpression) {
    this._result = new UnreadStatusQueryExpression(node.status);
  }

  visitDate(node: DateQueryExpression) {
    this._result = new DateQueryExpression(node.text, node.direction);
  }

  visitStarred(node: StarredStatusQueryExpression) {
    this._result = new StarredStatusQueryExpression(node.status);
  }

  visitHasAttachment(/* node */) {
    this._result = new HasAttachmentQueryExpression();
  }
}

/*
 * Converts a search query into the appropriate where clause. It does this by
 * converting match-compatible subtrees into the appropriate subquery that
 * uses a MATCH clause.
 */
class StructuredSearchQueryVisitor extends SearchQueryExpressionVisitor<string> {
  private _className: string;

  constructor(className: string) {
    super();
    this._className = className;
  }

  visit(root: QueryExpression) {
    return this.visitAndGetRequiredResult(root);
  }

  visitNot(node: NotQueryExpression) {
    const lhs = this.visitAndGetResult(node.e1);
    const rhs = this.visitAndGetResult(node.e2);
    this._result = `(${lhs} AND NOT ${rhs})`;
  }

  visitAnd(node: AndQueryExpression) {
    const lhs = this.visitAndGetResult(node.e1);
    const rhs = this.visitAndGetResult(node.e2);
    this._result = `(${lhs} AND ${rhs})`;
  }

  visitOr(node: OrQueryExpression) {
    const lhs = this.visitAndGetResult(node.e1);
    const rhs = this.visitAndGetResult(node.e2);
    this._result = `(${lhs} OR ${rhs})`;
  }

  visitFrom(node: FromQueryExpression) {
    throw new Error(`Unreachable: ${node}`);
  }

  visitTo(node: ToQueryExpression) {
    throw new Error(`Unreachable: ${node}`);
  }

  visitSubject(node: SubjectQueryExpression) {
    throw new Error(`Unreachable: ${node}`);
  }

  visitGeneric(node: GenericQueryExpression) {
    throw new Error(`Unreachable: ${node}`);
  }

  visitText(node: TextQueryExpression) {
    throw new Error(`Unreachable: ${node}`);
  }

  visitIn(node: InQueryExpression) {
    throw new Error(`Unreachable: ${node}`);
  }

  visitUnread(node: UnreadStatusQueryExpression) {
    this._result = `(\`${this._className}\`.\`unread\` ${node.status ? '>' : '='} 0)`;
  }

  visitStarred(node: StarredStatusQueryExpression) {
    this._result = `(\`${this._className}\`.\`starred\` ${node.status ? '>' : '='} 0)`;
  }

  visitHasAttachment(/* node */) {
    // Note: The sync engine populates attachmentCount in the data blob, but the
    // column is not queryable in the schema. This LIKE clause is a workaround.
    this._result = `(\`${this._className}\`.\`data\` NOT LIKE '%"attachmentCount":0%')`;
  }

  visitDate(node: DateQueryExpression) {
    const comparator = node.direction === 'before' ? '<' : '>';
    const date = DateUtils.getChronoPast().parseDate(node.text.token.s);
    if (!date) {
      this._result = '';
      return;
    }
    const ts = Math.floor(date.getTime() / 1000);
    this._result = `(\`${this._className}\`.\`lastMessageReceivedTimestamp\` ${comparator} ${ts})`;
  }

  visitMatch(node: MatchQueryExpression) {
    const searchTable = `${this._className}Search`;

    // in sqlite3, you use '' to escape a '. Weird right?
    const escaped = node.rawQuery.replace(/'/g, "''");

    this._result = `(\`${this._className}\`.\`id\` IN (SELECT \`content_id\` FROM \`${searchTable}\` WHERE \`${searchTable}\` MATCH '${escaped}'))`;
  }
}

export default class LocalSearchQueryBackend {
  private _modelClassName: string;

  constructor(modelClassName: string) {
    this._modelClassName = modelClassName;
  }

  compile(ast: QueryExpression) {
    const condenser = new MatchCompatibleQueryCondenser();
    const intermediateAST = condenser.visit(ast);

    const codegen = new StructuredSearchQueryVisitor(`${this._modelClassName}`);
    return codegen.visit(intermediateAST);
  }
}
