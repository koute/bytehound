import _ from "lodash";
import React from "react";
import { FormGroup, Label, Input, Button, Badge } from "reactstrap";
import { Link } from "react-router-dom";
import { ContextMenu, ContextMenuTrigger } from "react-contextmenu";
import classNames from "classnames";
import Feather from "./Feather.js";
import { fmt_date_unix_ms, fmt_date_timeval, fmt_uptime_timeval, format_frame, create_query } from "./utils.js";

const PERCENTAGE_REGEX = /^(\d+)%$/;
const DATE_REGEX = /^(\d{4})-(\d{1,2})-(\d{1,2})\s+(\d{1,2}):(\d{1,2}):(\d{1,2})(\.\d{3})?$/;
const SIZE_REGEX = /^(\d+)(k|m|g|t)?$/;
const DURATION_REGEX = /^(\d+d)?(\d+h)?(\d+m)?(\d+s)?(\d+ms)?(\d+us)?|\d+$/;
const POSITIVE_INTEGER_REGEX = /^(\d+)$/;

function validate_percentage( value ) {
    return value.match( PERCENTAGE_REGEX );
}

function validate_date( value ) {
    return value.match( DATE_REGEX );
}

function validate_or_percentage( alt ) {
    return (value) => {
        return validate_percentage( value ) || alt( value );
    };
}

function validate_positive_integer( value ) {
    return value.match( POSITIVE_INTEGER_REGEX );
}

function format_or_percentage( alt ) {
    return (value) => {
        if( validate_percentage( value ) ) {
            return value;
        } else {
            return alt( value );
        }
    };
}

function parse_or_percentage( alt ) {
    return (value) => {
        if( validate_percentage( value ) ) {
            return value;
        } else {
            return alt( value );
        }
    };
}

function date_to_unix_ms( value ) {
    const match = value.match( DATE_REGEX );
    const year = parseInt( match[ 1 ], 10 );
    const month = parseInt( match[ 2 ], 10 );
    const day = parseInt( match[ 3 ], 10 );
    const hour = parseInt( match[ 4 ], 10 );
    const minute = parseInt( match[ 5 ], 10 );
    const second = parseInt( match[ 6 ], 10 );
    const ms = parseInt( (match[ 7 ] || ".000").substr( 1 ), 10 );
    return Date.UTC( year, month - 1, day, hour, minute, second );
}

function validate_size( value ) {
    value = value.replace( " ", "" ).toLowerCase();
    return value.match( SIZE_REGEX );
}

function fmt_size_full( value ) {
    return value;
}

function parse_size( value ) {
    value = value.replace( " ", "" ).toLowerCase();
    const match = value.match( SIZE_REGEX );
    const unit = match[ 2 ];

    let mul = 1;
    if( unit === "k" ) {
        mul = 1000;
    } else if( unit === "m" ) {
        mul = 1000 * 1000;
    } else if( unit === "g" ) {
        mul = 1000 * 1000 * 1000;
    } else if( unit === "t" ) {
        mul = 1000 * 1000 * 1000 * 1000;
    }

    return parseInt( match[ 1 ], 10 ) * mul;
}

function validate_duration( value ) {
    value = value.replace( " ", "" ).toLowerCase();
    return value.length > 0 && value.match( DURATION_REGEX );
}

function parse_integer( value ) {
    return parseInt( value, 10 );
}

function identity( value ) {
    return value + "";
}

const DATE_FIELD = {
    kind: "entry",
    validate: validate_date,
    format: fmt_date_unix_ms,
    parse: date_to_unix_ms
};

const DATE_OR_PERCENTAGE_FIELD = {
    kind: "entry",
    validate: validate_or_percentage( DATE_FIELD.validate ),
    format: format_or_percentage( DATE_FIELD.format ),
    parse: parse_or_percentage( DATE_FIELD.parse )
};

const SIZE_FIELD = {
    kind: "entry",
    validate: validate_size,
    format: fmt_size_full,
    parse: parse_size
};

const POSITIVE_INTEGER_FIELD = {
    kind: "entry",
    validate: validate_positive_integer,
    format: identity,
    parse: parse_integer
};

const POSITIVE_INTEGER_OR_PERCENTAGE_FIELD = {
    kind: "entry",
    validate: validate_or_percentage( POSITIVE_INTEGER_FIELD.validate ),
    format: format_or_percentage( POSITIVE_INTEGER_FIELD.format ),
    parse: parse_or_percentage( POSITIVE_INTEGER_FIELD.parse )
};

const DURATION_FIELD = {
    kind: "entry",
    validate: validate_duration,
    format: identity,
    parse: identity
};

const DURATION_OR_PERCENTAGE_FIELD = {
    kind: "entry",
    validate: validate_or_percentage( DURATION_FIELD.validate ),
    format: format_or_percentage( DURATION_FIELD.format ),
    parse: parse_or_percentage( DURATION_FIELD.parse )
}

const RADIO_FIELD = {
    kind: "radio",
    validate: function( value ) { return value !== ""; },
    format: function( value ) { return value; },
    parse: function( value ) { return value; }
};

const REGEX_FIELD = {
    kind: "entry",
    validate: function( value ) {
        if( value === "" ) {
            return false;
        }

        try {
            new RegExp( value );
        } catch( exception ) {
            return false;
        }

        return true;
    },
    format: identity,
    parse: identity
};

function fmt_or_percent( formatter ) {
    return function( value ) {
        if( value.endsWith( "%" ) ) {
            return value;
        } else {
            return formatter( value );
        }
    };
}

function state_to_filter( fields, state ) {
    let output = {};
    if( !state ) {
        return output;
    }

    _.each( state, (raw_value, key) => {
        const field = fields[ key ];
        if( field.validate( raw_value ) ) {
            const value = field.parse( raw_value );
            output[ key ] = value;
        } else {
            output[ key ] = null;
        }
    });

    return output;
}

class FilterEditorBase extends React.Component {
    constructor( props, fields ) {
        super( props );
        this.fields = fields;
    }

    field( key ) {
        const fields = this.fields;
        const kind = fields[ key ].kind;
        const field = this.getFieldState( key );
        if( kind === "entry" ) {
            return (
                <div>
                    <Label for={key}>{fields[ key ].label}</Label>
                    <Input
                        id={key}
                        valid={field.is_changed}
                        invalid={field.is_invalid}
                        value={field.value}
                        onChange={e => this.onChanged( key, e.target.value )}
                    />
                </div>
            );
        } else if( kind === "radio" ) {
            const groups = _.map( fields[ key ].variants, (label, variant_key) => {
                return (
                    <FormGroup key={variant_key} check>
                        <Label check>
                            <Input
                                type="radio"
                                name={variant_key}
                                checked={field.value === variant_key}
                                onChange={e => {
                                    const new_value = e.target.value;
                                    if( new_value === "on" ) {
                                        this.onChanged( key, variant_key );
                                    }
                                }}
                            />{" "}
                            {label}
                        </Label>
                    </FormGroup>
                );
            });

            return (
                <FormGroup tag="div">
                    {groups}
                </FormGroup>
            );
        }
    }

    getFieldState( key ) {
        let original = this.props.filter[ key ];
        if( original === undefined || original === null ) {
            original = "";
        } else {
            original = this.fields[ key ].format( original );
        }

        let value = (this.props.state || {})[ key ];
        if( value === undefined || value === null ) {
            value = original;
        }

        const is_invalid = value !== "" && !this.fields[ key ].validate( value );
        const is_changed = !is_invalid && value !== original;
        return {
            value,
            original,
            is_invalid,
            is_changed
        };
    }

    onChanged( key, value ) {
        if( this.fields[ key ].clean ) {
            value = this.fields[ key ].clean( value );
        } else {
            value = value.trim().replace( / {2,}/, " " ).replace( /\s*:\s*/, ":" ).replace( /\s*-\s*/, "-" );
        }
        const new_state = {...this.props.state};
        new_state[ key ] = value;

        if( this.getFieldState( key ).original === value ) {
            delete new_state[ key ];
        }

        this.props.onChanged( new_state );
    }
}

class ControlBase extends React.Component {
    state = {}

    constructor( fields, filterEditor ) {
        super()
        this.fields = fields;
        this.filterEditor = filterEditor;
        this.toggleFilterEdit = this.toggleFilterEdit.bind( this );
    }

    static getDerivedStateFromProps( props, state ) {
        state.editedFilter = props.filter;
        return state;
    }

    render() {
        const badges = [];
        if( !_.isNil( this.props.totalCount ) ) {
            badges.push(
                <Badge key={0} className="filter-pill" color="primary" pill>Total: {this.props.totalCount}</Badge>
            );
        }

        const add = (value) => {
            badges.push(
                <Badge key={badges.length} className="filter-pill" color="info" pill>{value}</Badge>
            );
        };

        const filter = this.props.filter;
        _.each( this.fields, (field, key) => {
            const value = filter[ key ];
            if( _.isNil( value ) ) {
                return;
            }

            if( _.isFunction( field.badge ) ) {
                add( field.badge( value ) );
            } else {
                add( field.badge[ value ] );
            }
        });

        const editor = React.createElement( this.filterEditor, {
            filter: this.props.filter,
            state: this.state.filterEditorState,
            onChanged: this.onFilterChanged.bind( this )
        });

        return (
            <div className="navbar flex-column flex-md-nonwrap p-0 shadow w-100 px-3">
                <div className="d-flex justify-content-between w-100">
                    <div className="d-flex align-items-center">
                        <Link to="/" className="mr-3"><Feather name="grid" /></Link>
                        <Link to={"/overview/" + this.props.id} className="mr-3"><Feather name="bar-chart-2" /></Link>
                        <Link to={this.props.location} className="mr-3"><Feather name="anchor" /></Link>
                        <div>
                            {badges}
                        </div>
                    </div>
                    <div className="d-flex align-items-baseline py-1">
                        {this.renderLeft()}
                        <Label className="ml-2">Page</Label>
                        <Input className="ml-2" type="number" name="number" id="exampleNumber" placeholder="" style={{width: "6em"}} value={this.props.page + 1} onChange={e => this.onPageChange( e.target.value - 1 )} />
                        <Button outline color="primary" className="ml-2 btn-sm" onClick={this.toggleFilterEdit} style={{minWidth: "8em"}}>{
                            ( !this.state.editingFilter ) ? ("Change filter...") : ("Apply filter")
                        }</Button>
                        <div className="ml-3">
                            <ContextMenuTrigger id="allocations_navbar_context_menu" ref={c => this.menu_trigger = c}>
                                <Feather name="menu" onClick={event => this.menu_trigger.handleContextClick( event )} />
                            </ContextMenuTrigger>
                            <ContextMenu id="allocations_navbar_context_menu">
                                {this.renderMenu()}
                            </ContextMenu>
                        </div>
                    </div>
                </div>
                <div className={classNames( "pb-2 w-100 justify-content-between stretch-child", {"d-none": !this.state.editingFilter, "d-flex": this.state.editingFilter} )}>
                    {editor}
                </div>
            </div>
        );
    }

    toggleFilterEdit() {
        let filter_update = null;
        if( this.state.editingFilter ) {
            filter_update = state_to_filter( this.fields, this.state.filterEditorState );
        }

        this.setState({
            filterEditorState: null,
            editingFilter: !this.state.editingFilter
        });

        if( filter_update && !_.isEmpty( filter_update ) && this.props.onFilterChange ) {
            this.props.onFilterChange( {...this.props.filter, ...filter_update} );
        }

    }

    onFilterChanged( filterEditorState ) {
        this.setState( {filterEditorState} );
    }

    onPageChange( page ) {
        if( this.props.onPageChange ) {
            this.props.onPageChange( page );
        }
    }
}

function timestamp_cell( absolute, relative, relative_p ) {
    const percent = parseInt( relative_p * 100, 10 );
    return <div className="d-flex flex-row flex-column align-items-end"><div>{fmt_date_timeval( absolute )}</div><div>{fmt_uptime_timeval( relative )}, {percent}%</div><div></div></div>;
}

function backtrace_cell( show_full_backtraces, frames ) {
    let last_frame_index = frames.length - 1;

    while( frames[ last_frame_index - 1 ] !== undefined ) {
        const frame = frames[ last_frame_index ];
        if( frame.function ) {
            if( frame.function.match( /^(__gnu_cxx::|std::allocator_traits<|std::_Vector_base|std::_Deque_base|std::__allocated_ptr|std::__shared_count|std::__shared_ptr|boost::detail::shared_count|void boost::detail::sp_pointer_construct|std::string::_M_create|std::string::_M_mutate|std::string::_M_replace_aux|std::_Function_base)/ ) ) {
                last_frame_index -= 1;
                continue;
            }

            const m = frame.function.match( /::_M_([a-zA-Z0-9_]+)/ );
            if( m ) {
                const name = m[ 1 ].replace( /_aux$/, "" );
                if( frames[ last_frame_index - 1 ] && (frames[ last_frame_index - 1 ].function || "").includes( name ) ) {
                    last_frame_index -= 1;
                    continue;
                }
            }
        }

        break;
    }

    let indices = [last_frame_index, last_frame_index - 1];
    frames = frames.map((frame) => {
        let weight = 1.0;
        let shift = 0;
        if( frame.function ) {
            let d = frame.function.length - 120;
            if( d < 0 ) {
                d = 0;
            }
            weight += 0.3 * (d / 100);
            if( frame.function.match( /^(std|boost|void std)::/ ) ) {
                shift += 10;
            }
        }
        return {...frame, count: (frame.count * weight + shift) | 0 };
    });

    const minimums = {}
    for( let i = 0; i < last_frame_index - 2; ++i ) {
        const frame = frames[ i ];
        if( minimums[ frame.library ] === undefined || minimums[ frame.library ] > frame.count ) {
            minimums[ frame.library ] = frame.count;
        }
    }

    const min_count = _.min( _.map( frames, "count" ).slice( 0, last_frame_index - 2 ) );
    if( show_full_backtraces ) {
        for( let i = 0; i < frames.length; ++i ) {
            indices.push( i );
        }
    } else {
        for( let i = 0; i < last_frame_index - 2; ++i ) {
            const frame = frames[ i ];
            if( frame.count === minimums[ frame.library ] ) {
                indices.push( i );
            }
            if( frame.count === min_count ) {
                indices.push( i - 1 );
                indices.push( i - 2 );
            }
        }
    }

    indices.push( 0 );
    indices.sort((a, b) => a - b);
    indices = _.uniq( indices ).filter( index => index >= 0 && index < frames.length );

    let out = [];
    for( let i = 0; i < indices.length; ++i ) {
        let index = indices[ i ];
        const frame = frames[ index ];
        const entry = format_frame( index, frame );
        out.push( entry );
    }

    return out;
}

function get_data_url_generic( source_url, source, id, params ) {
    params = {...params};

    const page = (parseInt( params.page, 10 ) || 1) - 1;
    const page_size = parseInt( params.page_size, 10 ) || 20;

    params.count = page_size;
    params.skip = page_size * page;

    const encoded_body = create_query( _.omit( params, "page", "page_size" ) ).toString();
    const data_url = source_url + "/data/" + id + "/" + source + "?" + encoded_body;
    return data_url;
}

export {
    DATE_FIELD,
    DATE_OR_PERCENTAGE_FIELD,
    SIZE_FIELD,
    POSITIVE_INTEGER_FIELD,
    POSITIVE_INTEGER_OR_PERCENTAGE_FIELD,
    DURATION_FIELD,
    DURATION_OR_PERCENTAGE_FIELD,
    RADIO_FIELD,
    REGEX_FIELD,

    ControlBase,
    FilterEditorBase,

    fmt_or_percent,
    backtrace_cell,
    timestamp_cell,
    identity,
    get_data_url_generic,
}
