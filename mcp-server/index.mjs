#!/usr/bin/env node
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { registerTools } from './src/tools.mjs';

const VERSION = '0.1.2';

if (process.argv.includes('--version')) {
  process.stdout.write(`${VERSION}\n`);
  process.exit(0);
}

const server = new McpServer({
  name: 'deploy-desktop-mcp',
  version: VERSION
});

registerTools(server);

const transport = new StdioServerTransport();

process.on('uncaughtException', (error) => {
  console.error(`[deploy-desktop-mcp] uncaught exception: ${error.stack || error.message}`);
});
process.on('unhandledRejection', (error) => {
  console.error(`[deploy-desktop-mcp] unhandled rejection: ${error?.stack || error}`);
});

await server.connect(transport);

if (process.env.DEPLOY_MCP_DEBUG === '1') {
  console.error('[deploy-desktop-mcp] stdio server ready');
}
