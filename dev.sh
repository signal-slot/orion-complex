#!/bin/bash
# Start all orion-complex dev processes
set -e
cd "$(dirname "$0")"

# Kill existing processes
lsof -ti :2743 2>/dev/null | xargs kill 2>/dev/null || true
pkill -f orion-node-agent 2>/dev/null || true
sleep 1

# JWT token for the node agent
gen_token() {
  node -e "
    const c=require('crypto');
    const b=s=>Buffer.from(JSON.stringify(s)).toString('base64url');
    const hdr=b({alg:'HS256',typ:'JWT'});
    const now=Math.floor(Date.now()/1000);
    const pay=b({sub:'admin-1',iat:now,exp:now+86400*30});
    const sig=c.createHmac('sha256','dev-secret-change-in-production').update(hdr+'.'+pay).digest('base64url');
    console.log(hdr+'.'+pay+'.'+sig);
  "
}

NODE_ID=$(sqlite3 orion-complex.db "SELECT id FROM nodes WHERE name='mac-mini.local' LIMIT 1;" 2>/dev/null || echo "")

# 1. Build & start backend
echo "Starting backend..."
DATA_DIR=$HOME/.orion TLS_ENABLED=false cargo run &
sleep 3

# 2. Build, sign & start node agent
echo "Building node agent..."
cd macos-agent
swift build --product orion-node-agent 2>&1 | tail -3

echo "Signing node agent..."
cat > /tmp/vz-entitlements.plist << 'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>com.apple.security.virtualization</key>
	<true/>
</dict>
</plist>
PLIST
codesign --force --sign - --entitlements /tmp/vz-entitlements.plist .build/debug/orion-node-agent

# Force node online immediately so there's no wait
if [ -n "$NODE_ID" ]; then
  sqlite3 ../orion-complex.db "UPDATE nodes SET online = 1, last_heartbeat_at = $(date +%s) WHERE id = '$NODE_ID';"
fi

echo "Starting node agent..."
ORION_CONTROL_PLANE=http://127.0.0.1:2743 \
ORION_API_TOKEN="$(gen_token)" \
ORION_NODE_ID="$NODE_ID" \
.build/debug/orion-node-agent &

cd ..
echo ""
echo "Ready: https://localhost:2742"
echo "Press Ctrl+C to stop all"
wait
