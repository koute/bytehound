import _ from "lodash";
import React from "react";
import Graph from "./Graph.js";
import { Button, ButtonGroup, DropdownMenu, DropdownItem } from "reactstrap";
import { ContextMenu, MenuItem, ContextMenuTrigger } from "react-contextmenu";
import { Link } from "react-router-dom";
import classNames from "classnames";
import { extract_query, create_query, fmt_hex16, fmt_size, fmt_full_size } from "./utils.js";

export default class PageDataAddressSpace extends React.Component {
    state = { zoom: {} }

    constructor() {
        super()
        this.context_triggers = {};
    }

    componentDidMount() {
        const params = extract_query( this.props.location.search );
        const encoded_body = create_query( params ).toString();
        const url = (this.props.sourceUrl || "") + "/data/" + this.props.id + "/regions?" + encoded_body;
        fetch( url )
            .then( rsp => rsp.json() )
            .then( json => this.processData( json ) );
    }

    processData( raw_data ) {
        let data = [];
        let last = null;
        const regions = raw_data.regions;
        for( let i = 0; i < regions.length; ++i ) {
            let x0 = regions[ i ][ 0 ];
            let x1 = regions[ i ][ 1 ];

            if( last === null || x0 - last > 1024 * 1024 * 30 ) {
                data.push( {xs: [], ys: []} );
            }

            last = x0;
            const xs = data[ data.length - 1 ].xs;
            const ys = data[ data.length - 1 ].ys;
            xs.push( x0 - 0.0001 );
            ys.push( 0 );
            xs.push( x0 );
            ys.push( 1 );
            xs.push( x1 );
            ys.push( 1 );
            xs.push( x1 + 0.0001 );
            ys.push( 0 );
            data[ data.length - 1 ].allocated += x1 - x0;
        }

        const zoom = [];
        for( let i = 0; i < data.length; ++i ) {
            const xs = data[ i ].xs;
            const min = Math.round( xs[ 0 ] );
            const max = Math.round( xs[ xs.length - 1 ] );
            zoom.push( [min, max] );
        }

        this.setState( {data, zoom} );
    }

    render() {
        if( !this.state.data ) {
            return <div />;
        }

        const data = this.state.data;
        const output = [];
        for( let i = 0; i < data.length; ++i ) {
            const min = Math.round( this.state.zoom[ i ][ 0 ] );
            const max = Math.round( this.state.zoom[ i ][ 1 ] );

            output.push(
                <div key={"as_" + i}>
                <h2 className="h3">0x{fmt_hex16( min )} - 0x{fmt_hex16( max )} ({fmt_size( max - min )})</h2>
                <Graph
                    data={data[ i ]}
                    onZoom={this.onZoom.bind(this, i)}
                    onRightClick={this.onRightClick.bind(this, i)}
                    showYValue={false}
                    xUnit={x => fmt_hex16( Math.round( x ) )}
                    x_legend_formatter={x =>
                        fmt_hex16( Math.round( x ) ) + "<br />" + fmt_full_size( Math.round( x ) )
                    }
                    y_label=""
                    fill={true}
                    includeZero={false}
                    drawY={false}
                    fillAlpha={1.0}
                />
                <ContextMenuTrigger id={"context_menu_" + i} ref={c => this.context_triggers[ i ] = c}></ContextMenuTrigger>
                <ContextMenu id={"context_menu_" + i}>
                    <MenuItem>
                        <Link to={this.allocationsRangeLink( i )}>Allocations in {this.getSelectedRange( i )}</Link>
                    </MenuItem>
                </ContextMenu>
                </div>
            );
        }

        return (
            <div className="PageDataAddressSpace pt-3 px-4">
                <h1 className="h2">Address space fragmentation</h1>
                {output}
            </div>
        );
    }

    allocationsRangeLink( i ) {
        if( this.state.context_range === undefined ) {
            return "/";
        }

        const x0 = Math.round( this.state.context_range[ 0 ] );
        const x1 = Math.round( this.state.context_range[ 1 ] );
        const params = extract_query( this.props.location.search );
        params.address_min = x0;
        params.address_max = x1;
        const encoded_body = create_query( params ).toString();
        return "/allocations/" + this.props.id + "?" + encoded_body;
    }

    getSelectedRange( i ) {
        if( this.state.context_range === undefined ) {
            return "";
        }

        const x0 = Math.round( this.state.context_range[ 0 ] );
        const x1 = Math.round( this.state.context_range[ 1 ] );
        return "0x" + fmt_hex16( x0 ) + " - " + "0x" + fmt_hex16( x1 );
    }

    onZoom( nth, min, max ) {
        const zoom = _.clone( this.state.zoom );
        zoom[ nth ] = [min, max];

        this.setState( {zoom} );
    }

    onRightClick( i, {event, x, x0, x1} ) {
        this.setState( {context_x: x, context_range: [x0, x1]} );
        this.context_triggers[ i ].handleContextClick( event );
    }
}
