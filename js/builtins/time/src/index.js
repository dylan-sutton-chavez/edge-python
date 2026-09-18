import clock from './clock.js';
import fmt from './fmt.js';

/* Factory consumed by createWorker({ mainThreadModules: { time } }). Composes the clock + fmt slices. */
export const time = () => Object.assign({}, clock(), fmt());

export default time;
