// Runs DatabaseStore's background queries in a utility process (main/ipc/database-agent.ts):
// the first message from main carries the port to the window, which sends the queries on it.
const Sqlite3 = require('better-sqlite3');
const dbs = {};

const deathDelay = 5000;
let deathTimer = setTimeout(() => process.exit(0), deathDelay);

function getDatabase(dbpath) {
  if (dbs[dbpath]) {
    return Promise.resolve(dbs[dbpath]);
  }

  try {
    dbs[dbpath] = new Sqlite3(dbpath, { readonly: true, timeout: 10000 });
  } catch (err) {
    console.error(err);
    process.exit(1);
  }

  return Promise.resolve(dbs[dbpath]);
}

function onQuery(m, reply) {
  clearTimeout(deathTimer);
  const { query, values, id, dbpath } = m;
  const start = Date.now();

  getDatabase(dbpath)
    .then(db => {
      clearTimeout(deathTimer);
      const fn = query.startsWith('SELECT') ? 'all' : 'run';
      const stmt = db.prepare(query);
      const results = stmt[fn](values);
      reply({ type: 'results', results, id, agentTime: Date.now() - start });
    })
    .catch(err => {
      // The window runs the query itself, with its retries for a locked or changed schema.
      console.error(err);
      reply({ type: 'error', id });
    })
    .finally(() => {
      clearTimeout(deathTimer);
      deathTimer = setTimeout(() => process.exit(0), deathDelay);
    });
}

process.parentPort.once('message', ({ ports: [port] }) => {
  port.on('message', ({ data }) => onQuery(data, message => port.postMessage(message)));
  port.start();
});
