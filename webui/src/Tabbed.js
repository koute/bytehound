import _ from "lodash";
import React from "react";
import { Nav, NavItem, NavLink, TabContent, TabPane } from "reactstrap";
import { Link } from "react-router-dom";
import classNames from "classnames";

export default class Tabbed extends React.Component {
    state = { active: null }

    render() {
        let children = this.props.children;
        if( !_.isArray( children ) ) {
            children = [children];
        }

        const active = this.state.active || children[ 0 ].key || 0;

        let nav = [];
        let tabs = [];
        _.each( children, (child, index) => {
            const key = child.key || index;
            const child_nav = (
                <NavItem key={key}>
                    <NavLink
                        className={classNames({ active: active === key })}
                        onClick={() => { this.setState( {active: key} ) }}
                    >
                        {child.props.title}
                    </NavLink>
                </NavItem>
            );

            const inner = React.cloneElement( child, {title: null} );
            const child_tab = (
                <TabPane key={key} tabId={key}>
                    {inner}
                </TabPane>
            );

            nav.push( child_nav );
            tabs.push( child_tab );
        });

        return (
            <div>
                <Nav tabs>
                    {nav}
                </Nav>
                <TabContent activeTab={active} className="py-2">
                    {tabs}
                </TabContent>
            </div>
        );
    }
}
