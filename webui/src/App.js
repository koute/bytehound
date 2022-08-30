import React from "react";
import { Route } from "react-router-dom";
import _ from "lodash";

import PageDataList from "./PageDataList.js";
import PageDataOverview from "./PageDataOverview.js";
import PageDataAllocations from "./PageDataAllocations.js";
import PageDataAddressSpace from "./PageDataAddressSpace.js";
import PageDataConsole from "./PageDataConsole.js";
import PageDataMaps from "./PageDataMaps.js";

export default class App extends React.Component {
    render() {
        return (
            <div className="App">
                <main role="main" className="w-100">
                    <Route exact path="/overview/:id" render={ ({ match, location, history }) => {
                        return <PageDataOverview key="overview" location={location} sourceUrl={this.props.sourceUrl} id={match.params.id} />;
                    }} />
                    <Route exact path="/allocations/:id" render={ ({ match, location, history }) => {
                        return <PageDataAllocations key="allocations" location={location} history={history} sourceUrl={this.props.sourceUrl} id={match.params.id} />;
                    }} />
                    <Route exact path="/maps/:id" render={ ({ match, location, history }) => {
                        return <PageDataMaps key="maps" location={location} history={history} sourceUrl={this.props.sourceUrl} id={match.params.id} />;
                    }} />
                    <Route exact path="/address_space/:id" render={ ({ match, location, history }) => {
                        return <PageDataAddressSpace key="address_space" location={location} history={history} sourceUrl={this.props.sourceUrl} id={match.params.id} />;
                    }} />
                    <Route exact path="/console/:id" render={ ({ match, location, history }) => {
                        return <PageDataConsole key="console" location={location} sourceUrl={this.props.sourceUrl} id={match.params.id} />;
                    }} />
                    <Route exact path="/" render={ () => {
                        return <PageDataList key="list" sourceUrl={this.props.sourceUrl} />;
                    }} />
                </main>
            </div>
        );
    }
}
