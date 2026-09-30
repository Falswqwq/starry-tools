/** 前端测试的统一入口：`npm test` 会把它打包再交给 node 跑。 */

import './lib/dev-render.test';
import './lib/dev-snapshot.test';
import './lib/graph.test';
import './lib/geometry.test';
import './lib/params.test';
import './graph/ParamField.test';
import './graph/ToolNodeView.test';
import './graph/WireEdge.test';
import './panels/NodeLibrary.test';
import './panels/Chrome.test';
import './ui/Select.test';
